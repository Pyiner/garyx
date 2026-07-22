# Garyx Agent Guide

This is the short repo-level guide for coding agents. Keep only the core
operating principles here; detailed rules live in the referenced files under
`docs/agents/`.

## Core Rules

- Read the local code around a change before editing.
- Keep changes scoped to the smallest correct surface and preserve unrelated
  user work in the worktree.
- Prefer existing crate, SwiftUI, Electron, and UI patterns over new
  abstractions unless the change clearly needs one.
- Run focused deterministic validation for the touched area.
- Architecture guards are structural, never textual: enforce boundaries
  with visibility, capability tokens, feature-gated test constructors, and
  importable contract constants — never with tests that regex-scan source
  files. `cfg(test)` must never replace production behavior; test seams
  are additive, with the production side wired explicitly at the
  composition root.
- Commit completed code changes before handoff. Stage only files changed for
  the current task.
- When pushing completed work, push directly to the remote `main` branch unless
  the requester explicitly specifies a different target branch.
- Dedicated development worktrees are temporary and owned by the task creator
  or orchestrator. After explicit approval, preserve the required commits,
  remove the worktree from its parent checkout, verify the Git worktree record
  is gone, and only then mark the task done. A done task must not leave its
  worktree or Rust `target` cache behind. Never remove an active worktree or
  discard unpreserved changes to satisfy cleanup.
- When committing from an agent, use the repository's configured Git author and
  committer metadata. Do not override local Git identity.
- When you create review tasks for your own work, notify `current-thread` so
  review results return to the task thread instead of a personal bot channel.
- Self-review is not enough for adversarial review gates. Assign review tasks to
  a different model family than the implementer, for example `--agent claude`
  for Codex-authored changes.

## Public Repository Hygiene

This is a public repository. Never commit real personal data in fixtures, docs,
code samples, or commit messages. That includes real names, chat/user IDs, bot
IDs, email addresses, phone numbers, `/Users/<username>` paths, tokens, and
secrets.

Use clearly synthetic placeholders such as `Test User`, `1000000001`,
`/Users/test`, `bot@example.com`, and `${TOKEN}`. Before staging, scan the diff
for personal data and remove it.

## Repository Map

- `garyx`: CLI entrypoint and runtime assembly.
- `garyx-gateway`: HTTP API, MCP server, automations, restart flow, and desktop
  surface.
- `garyx-router`: canonical threads, transcripts, endpoint state, and routing.
- `garyx-bridge`: provider orchestration for Claude Code, Codex, Traex, and
  Antigravity.
- `garyx-channels`: built-in channel runtimes and subprocess plugin host.
- `desktop/garyx-desktop`: Electron desktop app and shared renderer UI.
- `mobile/garyx-mobile`: iOS app, widget, and `GaryxMobileCore` Swift package.

## Product And Data Contracts

- Config lives in `~/.garyx/garyx.json`; gateway/router state is the source of
  truth for persisted runtime data.
- Thread records live in the `thread_records` SQLite table (truth source;
  #TASK-1864); conversation content lives in transcript jsonl. Projections
  derive in the same transaction as every record write — never repaired by
  read routes, backfills, or reconciles. All thread condition queries go
  through SQL projections.
- Workspace identity is always the absolute directory path string. Do not add
  workspace IDs or infer root workspace rows from thread metadata.
- `No workspace` means the user did not choose a workspace; runtime threads must
  still get a private Garyx-managed thread workspace and provider cwd must match
  the thread workspace.
- Desktop and mobile root workspace lists come from gateway `/api/workspaces`
  application state.
- Configured bot account `config` is ordinary application state; do not add
  token-specific merge, redaction, or preservation paths beyond keeping real
  secrets out of committed fixtures.
- Prompt attachments never use timer-based cleanup. Retain unreferenced staging
  uploads unless their own failed upload transaction is rolled back. Attachments
  referenced by committed messages are durable conversation content: never
  expire or delete them on upload TTL, claim lease expiry, provider-run
  termination, or thread archive. Ownership follows the thread, and physical
  deletion runs only through the thread cleanup outbox after thread deletion.
- A committed migration marker makes that migration's normalization contract
  durable protocol, not dead code. Never weaken it when retiring runtime
  fields; repair already-marked databases with a new versioned migration.
- `/api/threads/history` preserves message string segments and text/content
  summaries without size truncation; larger history responses are accepted.
  Do not reintroduce caps for user, assistant, or nested tool text.
- Provider, agent, bot, and channel identity presentation should resolve
  through shared presentation helpers instead of local view switch tables.
- Provider identity surfaces must reuse the same avatar component and branded
  artwork as Agent surfaces; do not substitute screen-local SF Symbols.
- A Claude Code `result` is a turn boundary, not a process boundary. Normal
  completion may close stdin, but must consume stdout through EOF and wait for
  natural process exit; background-task level signals never authorize output
  teardown.

Detailed data and runtime contracts: @docs/agents/repository-contracts.md and
@docs/agents/workspace-paths.md.

## UI Direction

- The Mac app is the source of truth for information architecture, labels,
  field meaning, icon semantics, and Gateway-backed data models.
- Mobile may adapt layout and interaction for iOS, but must not invent new
  top-level concepts.
- Garyx iOS product work targets only the latest iOS release (currently iOS
  26). Validate iOS changes against a single reference configuration: the
  iPhone 17 Pro Max device size on iOS 26.5. Do not add or validate
  compatibility behavior for earlier iOS versions, other iOS builds, or other
  device sizes unless explicitly requested; leave existing fallback code
  unchanged when it is outside the task scope.
- iOS visual QA is light-mode only; dark mode is not a supported target. Do
  not add dark-mode support or run dark-mode test passes unless the user
  explicitly requests them; leave existing dark-mode behavior unchanged when
  it is outside the task scope.
- Use native platform patterns: Electron/shadcn-style desktop surfaces where
  appropriate, and native grouped iOS management surfaces on mobile.
- Provider default model and reasoning labels should use the row's available
  control width. Do not impose percentage caps that truncate short names when
  the row has room to show them in full.
- Electron `contextBridge` exposes `window.garyxDesktop` as a frozen
  cross-context object. Never target it directly with a Proxy that substitutes
  property values; materialize intercepting methods on a separate facade.
- Desktop app-shell chrome must keep its complete geometry and state recipe in
  an always-loaded owner stylesheet. Never place global shell component rules
  in a removable feature stylesheet; pin the owner import with a contract test.
- Desktop thread side tools have one presentation: a right-docked rail. The
  header control toggles that rail directly; do not add an overlay variant.
- Desktop thread logs are a built-in tool inside the right-docked side-tools
  rail and inherit that rail's responsive behavior. Do not add independent log
  occupancy, funding, dock/overlay placement, resizers, or width preferences.
  The side-tools Tasks item is removed; the global Tasks entry and route and
  the thread task tree remain separate features.
- Desktop responsive collapse order is the secondary conversation rail at
  980px, then the global sidebar at 720px. An explicit compact-sidebar open is
  an in-flow native-window expansion, never an overlay. Funded panel closes
  publish the closed frame before shrinking the native window on the next
  frame; do not add a fixed delay without a real matching transition.
- Mobile route state, presentation mapping, formatting, and business-rule
  transformations should live in `GaryxMobileCore` with SwiftPM tests.
- Mobile Provider overviews use one expanded identity, quota, and defaults
  composition for every built-in provider. Claude Code alone inserts the
  account-selection row; providers without metered usage keep the shared quota
  row and show `No quota data` instead of falling back to a compressed style.
- Mobile conversation pushes must present complete, production-identical
  thread chrome from the first destination frame; route-level plain or
  full-page loading covers are forbidden. Keep existing local transcript rows
  visible during refresh, use the shared message-region loading view only when
  there are zero local renderable rows, and preserve the shared header loading
  spinner. Prewarm the exact shared components and retain both moving-transition
  and post-reveal hitch gates.
- Mobile thread-list rows never expose swipe actions. Keep row actions in the
  existing long-press context menu, with destructive actions presented as such.
- Message, transcript, and tool-row display is server-render-state first:
  `garyx-models` derives `render_state` from the committed event ledger and the
  gateway sends it in per-thread `thread_render_frame` SSE. Desktop and mobile
  may map that snapshot into platform view models, but must not recompute
  transcript rows, tool grouping, active tool state, final-answer placement, or
  tail thinking locally. `render_state.rows` may be narrowed by a
  client-declared `render_floor`; `based_on_seq` remains the committed window
  tail and event delivery is still governed by the SSE cursor. Prioritize
  headless, no-UI tests for message-related work by driving the server snapshot
  / real captured stream data and asserting the mapped output.
- Tool call/result field selection is server-owned through
  `RenderToolEntry.projection`. Keep projections lightweight by sending field
  selectors, not copied command output; Mac and iOS resolve those selectors
  generically against their cached message bodies and must not add separate
  provider-specific JSON field switch tables.
- Desktop `RichMessageText` uses Streamdown, which wraps Markdown blocks in
  `display: contents` containers. First/last-block CSS must target the logical
  message root; broad descendant selectors such as `p:last-child` can match
  every wrapped paragraph and accidentally remove all paragraph spacing.
- Keep mobile SwiftUI feature surfaces in feature-specific files.
- Mobile page backgrounds and bottom floating controls should use the shared
  safe-area chrome helpers (`garyxPageBackground`, `garyxFloatingBottomChrome`)
  instead of local `ignoresSafeArea` / `Color.clear` patches.
- Keep SwiftUI presentation-anchor structure stable while lease or modal-barrier
  state changes. Gate actions inside an always-attached modifier instead of
  conditionally replacing modifier branches, which can tear down an in-flight
  system presentation.
- Keep custom SwiftUI presentation `Binding` getters pure. Acquire and settle
  modal leases from always-attached lifecycle callbacks, binding setters, or
  dismissal callbacks, and make repeated dismissal observation idempotent.
- `UIViewRepresentable` and `UIViewControllerRepresentable` lifecycle
  callbacks — make, update, and dismantle — run inside SwiftUI graph updates;
  dismantle can run inside graph teardown while a window or scene deallocates.
  Never synchronously publish from any of these callbacks into SwiftUI-observed
  storage: a `@Published` write there can re-enter the graph and abort with a
  Swift exclusivity violation. Keep imperative UIKit lifecycle controllers in
  stable, non-observable reference state, attach UIKit observation from UIKit
  hierarchy callbacks, and return business outcomes through explicit callbacks.
  Detach/teardown bookkeeping that must settle observable state defers that
  publish outside the current graph update.
- Occurrence-scoped async route preparation must release waiters immediately
  when an occurrence is superseded. A stale completion may clean up its own
  waiters, but must never mutate the current occurrence bookkeeping.

Detailed UI rules: @docs/agents/mobile-ui.md and @docs/agents/desktop-ui.md.

## Release And Runtime Boundaries

- Claude Code account selection is provider-owned runtime state. Do not persist
  `CLAUDE_CONFIG_DIR` in thread or agent metadata; snapshot the provider's
  selected environment only when a new top-level run starts.
- Quota recovery is SQL-owned per blocked run generation. Timer, account
  switch, and manual Continue must wake the same durable row and admission
  intent; never queue its synthetic `continue` into an active run, and never
  let a surviving legacy `quota-resend:` cron dispatch independently.
- Anthropic OAuth usage can return a valid Fable `weekly_scoped` allowance with
  `is_active: false`; that flag means the bucket is not currently consuming,
  not that its quota is unavailable. Preserve scoped limits that have a usable
  model scope and percentage.
- Claude quota reads must not rotate a still-valid access token. Refresh stored
  OAuth credentials only after `expiresAt` has elapsed or after the usage API
  rejects the access token with 401, retry at most once, and atomically persist
  any rotated refresh token before using the replacement credential. Network,
  rate-limit, and upstream failures never trigger credential refresh.
- Gateway code changes do not affect the running gateway until the binary is
  built, installed, and the managed gateway is restarted.
- For local macOS gateway development, use `scripts/build-local-cli.sh` to
  produce a `target/release/garyx` signed as `com.garyx.gateway`; use
  `scripts/install-local-cli.sh` when that binary also needs to be copied into
  every local CLI path currently in use. Do not manually copy a raw Cargo build
  over an installed `garyx`.
- iOS TestFlight releases are independent from macOS/gateway release flow. Do
  not trigger TestFlight unless the user explicitly asks in the current turn.
- Do not wire iOS uploads into version-tag release jobs unless the user
  explicitly asks for that coupling.
Detailed runtime and SDK rules:
@docs/agents/gateway-runtime.md, @docs/agents/claude-sdk.md, and
@docs/agents/working-loop.md.

## Validation

Use the narrowest reliable validation for the touched area. Common commands and
fallbacks live in @docs/agents/validation.md.

Keep a separate Cargo `target` directory in every concurrent Rust worktree.
The checked-in `.cargo/config.toml` disables incremental/full dev-test debug
artifacts and uses the worktree-aware sccache wrapper; do not replace it with a
single shared `CARGO_TARGET_DIR`.

For Rust changes, prefer the fast local loop before any full workspace run:
`scripts/test/rust_tier1_fast.sh --changed`. For focused work, run the touched
crate or exact test directly, for example `cargo test -p garyx-gateway --lib`,
`cargo test -p garyx-router --all-targets`, or
`cargo test -p garyx-gateway some_exact_test_name --lib -- --exact --nocapture`.
Use `RUST_TEST_FAIL_FAST=1` when a quick first failure is more useful than a
complete report. Reserve `scripts/test/rust_tier2_pr.sh` for PR-ready or
cross-crate validation, and
`RUN_EXTERNAL_AI_TESTS=1 scripts/test/rust_tier3_extended.sh` for
ignored/external-provider integration coverage.

For fast Mac app iteration, it is fine to run the desktop app in dev mode and
attach with CDP while working through UI behavior. Before handoff, still do one
packaged-app check when renderer resources, preload IPC, app bundling, or
installed-app behavior changed.

For real Mac app renderer checks after desktop changes, run
`npm run dist:dir` in `desktop/garyx-desktop`, quit any stale `Garyx` process,
open the installed app, then attach with
`playwright-cli -s=<session> attach --cdp=http://127.0.0.1:39222`. Restarting
the app before attaching avoids testing an old renderer bundle.

## Keep AGENTS.md And CLAUDE.md In Sync

The repo-level `AGENTS.md` and `CLAUDE.md` are intentionally identical so every
coding agent gets the same guidance. When changing one, change the other in the
same commit. `AGENTS.md` is the authoritative source; `CLAUDE.md` is the mirror
copy.
