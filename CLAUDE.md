# Garyx Agent Guide

This file is the short repo-level guide for coding agents.

## IMPORTANT: No real personal data in committed content

This is a public repository. NEVER use real personal data in test fixtures,
docs, code samples, or commit messages. That includes — but is not limited to —
real names, real Telegram/WeChat/Feishu chat IDs and user IDs, real bot IDs,
real email addresses, real phone numbers, real `/Users/<username>` paths, and
real tokens or secrets. Use clearly synthetic placeholders instead, e.g.
`Test User`, `1000000001`, `/Users/test`, `bot@example.com`, `${TOKEN}`.

Before staging a change, scan the diff for personal data and remove it. If a
test needs to reference an account, build it from a placeholder fixture, not
from a real chat captured during local debugging.
When committing from an agent, use the repository's configured Git author and
committer metadata. Do not override the local Git identity with placeholder
names or emails.

## Repository Shape

- `garyx`: CLI entrypoint and runtime assembly.
- `garyx-gateway`: HTTP API, MCP server, automations, restart flow, and desktop surface.
- `garyx-router`: canonical threads, transcripts, endpoint state, and routing.
- `garyx-bridge`: provider orchestration for Claude Code, Codex, Gemini, and teams.
- `garyx-channels`: built-in channel runtimes and subprocess plugin host.
- `desktop/garyx-desktop`: Electron desktop app and shared renderer UI.

## Source of Truth

- Config: `~/.garyx/garyx.json`.
- Channel accounts: `channels.<channel_id>.accounts[...]` (`channels.api.accounts[...]` for API).
- Configured bot account `config` is ordinary application state. Mobile and
  desktop should not add token-specific merge, redaction, or preservation paths
  beyond keeping real secrets out of committed fixtures.
- Thread records and known endpoint state: `garyx-router`.
- Mobile recent-thread lists read the gateway SQLite `recent_threads` projection
  only. Keep that projection current by writing it from the thread-store write
  path; do not make `GET /api/recent-threads` rescan router/thread files.
- Provider bridge run persistence must use the same projecting thread store as
  the gateway/router so active run state updates are dual-written into
  `recent_threads` at write time. Do not repair stale `active_run_id` or
  `run_state` in read routes.
- Startup reconciliation may repair historically stale active-run projection
  rows as a data migration, but steady-state correctness must come from the
  thread-store write path.
- MCP schema and tool behavior: `garyx-gateway/src/mcp.rs`.
- Provider session behavior: `garyx-bridge`.
- Channel/plugin stream presentation policy, buffering, and tool-call display
  helpers: `garyx-channels/src/plugin_tools.rs`.
- Telegram uses the throttled plugin stream policy: assistant text can stream
  through 300ms edit coalescing while top-level tool calls flush immediately.
- Discord uses the buffered plugin stream policy: assistant text deltas wait
  until a top-level tool call starts or the run finishes; rapid tool-call
  placeholder updates are coalesced with a one-second minimum interval. When a
  queued user message is acknowledged mid-stream, Discord finalizes the current
  reply segment and starts subsequent assistant output in a new message.
  Discord REST writes retry 429, transient network, and 5xx responses with
  backoff.
- Garyx in-process native model providers load Garyx-managed Skills from
  `~/.garyx/skills` and managed MCP from gateway-injected
  `remote_mcp_servers`; they should not read downstream Claude/Codex Skill or
  MCP config files.

## Workspace And Path Model

- Workspace identity is the absolute directory path everywhere: desktop,
  mobile, gateway, and CLI flows must pass and persist the path string directly.
  Do not add workspace IDs; workspaces are directory filters/bookmarks, not a
  separate domain entity.
- Desktop and mobile root workspace lists must contain only user-added
  workspaces persisted by gateway `/api/workspaces` application SQLite state.
  Thread `workspacePath` values and temporary workspace paths are metadata only;
  they may help with sorting, file-link resolution, or form suggestions, but
  must not create inferred root workspace rows.
- If the gateway workspace table has no rows, gateway initialization may seed it
  once from configured bot accounts and scheduled automation jobs. Soft-deleted
  rows count as existing state and must prevent future inferred reseeding.
- Desktop and mobile workspace fields should expose a platform-local shared
  workspace select. Options come from `/api/workspaces`; selected business
  values are always absolute path strings. If a current path is no longer in the
  saved list, keep displaying and submitting it unchanged until the user picks a
  different workspace.
- The final workspace select item should be `Add workspace`. It opens a
  lightweight directory browser that shows the current folder, immediate child
  folders, back navigation, and an explicit "use this folder" action. Add through
  the backend workspace API, refresh options, and set the form field to the
  added absolute path. Do not use native file-manager dialogs or raw path inputs
  as the primary control in ordinary forms.

## Claude Code SDK Notes

- In bidirectional Claude Code SDK sessions, every CLI `control_request` must
  receive a `control_response`, even when the subtype is unsupported or newer
  than this SDK. Dropping one can leave the CLI waiting forever.
- Normal streaming completion should close stdin and wait for the Claude CLI
  process to exit; force-closing the transport can race Claude's local
  transcript flush and break later `--resume` behavior.
- Claude SDK approval tests need a `can_use_tool` callback, which adds
  `--permission-prompt-tool stdio`; changing only `permission_mode` may not
  make Claude send `can_use_tool` requests to the SDK.
- The standalone Claude SDK should preserve Claude Code's built-in system
  prompt by default. Pass `--system-prompt` only when Garyx intentionally
  replaces it; use `--append-system-prompt` to keep default tool behavior.

## Working Loop

1. Read the local code around the change before editing.
2. Keep the change scoped to the smallest correct surface.
3. Prefer existing crate and UI patterns over new abstractions.
4. Run focused deterministic tests for the touched area.
5. When touching the macOS app under `desktop/garyx-desktop`, start the dev
   client and use it for user-facing previews unless the user explicitly asks
   for a packaged app or the change affects packaging, install, release, or
   startup behavior.
6. For runtime changes that affect the managed gateway's behavior, install the
   new binary and run a synthetic end-to-end CLI smoke test against the running
   gateway after unit tests pass.
7. Update [docs/configuration.md](docs/configuration.md) when user-facing
   configuration, CLI flags, environment variables, config-file schema, or setup
   behavior changes.
8. Commit every completed code change before handoff. Stage only the files changed
   for the current task, and leave unrelated user work untouched.
9. iOS TestFlight releases are independent from the macOS/gateway release flow:
   do not wire iOS uploads into version-tag release jobs unless the user
   explicitly asks for that coupling. Do not trigger an iOS TestFlight build
   or upload unless the user explicitly asks for TestFlight packaging/upload in
   the current turn.

## Desktop Dev Mode

For fast macOS UI iteration, run the Electron dev build:

```bash
cd desktop/garyx-desktop && npm run dev
```

This launches the Garyx Mac app in development mode. Renderer changes are visible
directly in the running Mac app as you edit, so use this mode for quick visual
and interaction feedback. Prefer showing the user this dev client during normal
UI iteration because code changes take effect without rebuilding a package.
Packaging is optional unless the user asks for a packaged app or the change
needs installed-app validation.

## Mobile UI

- The Mac app is the source of truth for mobile information architecture,
  labels, field meaning, icon semantics, and Gateway-backed data models. Mobile
  may adapt layout and interaction for iOS, but must not invent new top-level
  concepts.
- Use native iOS patterns for management surfaces: grouped lists, compact rows,
  top navigation actions, segmented controls for peer categories, and row-level
  ellipsis menus for secondary actions. Do not port desktop card/action-bar
  layouts directly into mobile.
- Use Garyx's existing adaptive glass/material helpers for mobile chrome. Keep
  content readable, near-white, and integrated; reserve glass for navigation and
  transient controls rather than repeated content rows.
- Mobile top-left controls and leading-edge gestures must share the same route
  action. Direct sidebar children may open the sidebar; deeper pages go back to
  their immediate parent.
- Mobile entry points that open an existing thread by row tap, widget link,
  task, automation, bot conversation, or deep link should route through the
  shared `GaryxMobileModel.openThread` path; sidebar behavior is the baseline.
- Mobile sidebar root navigation shows Automation and Workspace & Bots; Tasks,
  Auto Research, Agents, and Skills live under Settings. Keep workspace and bot
  conversations inside drilldown layers rather than dumping raw sessions inline.
- Mobile widgets are static snapshots: do not use `ScrollView`; start directly
  with thread rows, render pinned rows like other rows, and use agent/team
  avatars where available.
- Recent-thread widget row taps must use per-row `Link` destinations only. Do
  not attach a container `.widgetURL` to the first thread because it can steal
  row taps and open the wrong conversation.
- Provider, agent, team, bot, and channel identity presentation must resolve
  through shared Core presentation helpers; do not add local switch tables in
  views, widgets, or settings.
- Mobile chat, transcript, automation, widget, and workspace/bot visual details
  live in the `garyx-product-ui` skill. Use that skill for non-trivial mobile UI
  implementation or review.
- Pure mobile route state, presentation mapping, formatting, and business-rule
  transformations should live under `mobile/garyx-mobile/Sources/GaryxMobileCore`
  with SwiftPM tests. Keep app-target files focused on SwiftUI composition,
  bindings, platform adapters, and side-effect orchestration.
- Mobile low-frequency catalog data such as agents, teams, workspaces, bots,
  skills, automations, tasks, slash commands, and MCP servers should use
  gateway-scoped stale-while-refresh caching. Restored rows are display
  projections only; edit paths that preserve hidden gateway fields must fetch
  authoritative data before saving.
- Keep mobile SwiftUI feature surfaces in feature-specific files rather than
  adding large view trees to `GaryxMobileViews.swift`.

## Desktop Transcript And File Tree Rules

- Treat transcript history as user-turn based: one user message plus the
  following agent activity until the next user message. Pagination, prefetch,
  folding, and final-answer visibility should use that unit rather than raw
  provider messages or tool-call counts.
- Keep completed user-turn final answers visible when turns collapse. While a
  thread is still running, keep active turn containers stable and reserve
  Working/Worked rows for real tool activity; pure assistant/reasoning text
  remains normal assistant text.
- Desktop interruption controls must be gateway-backed. The local Mac app
  process may not own the active WebSocket for runs started elsewhere or after
  a reload; after trying any local active socket, call the gateway chat
  interrupt endpoint so the bridge can interrupt or abort the active thread run.
- The workspace file browser should read directories on demand. Do not pre-scan
  child directories just to decide whether to show expansion affordances,
  especially on macOS where probing protected folders can trigger privacy
  prompts.
- Desktop chat, transcript, workspace selector, and file-tree interaction
  details live in the `garyx-product-ui` skill. Use that skill for non-trivial
  desktop UI implementation or review.

## Gateway Runtime

- Code changes do not affect the running gateway until the binary is built,
  installed, and the managed gateway is restarted.
- On macOS, do not treat a matching hash as sufficient after copying a locally
  built `garyx` binary into a launchd-managed path such as
  `/opt/homebrew/bin/garyx`. Clear removable target-file xattrs, ad-hoc re-sign
  the installed file with the stable identifier `com.garyx.gateway` (or use
  `bash scripts/codesign-macos-cli.sh <path-to-garyx>`), and verify it executes
  before restarting, otherwise launchd/AMFI may kill it with
  `OS_REASON_CODESIGNING`. `com.apple.provenance` can be inherited or protected
  on Homebrew paths even when `xattr -d` returns success, so do not rely on
  xattr output alone.
- For local macOS gateway development, prefer `scripts/install-local-cli.sh`
  after source changes. Release archives, `install.sh`, `garyx update`, and
  desktop `build:rust` should all preserve the same CLI identifier so directory
  authorization is not re-requested just because a new binary was installed.
  `install.sh` installs the signed release binary as-is and must not re-sign it
  after download.
- Restart through the Garyx CLI. When continuation is needed in an active agent
  thread, queue a wake, for example `garyx gateway restart --wake thread
  <thread_id> --wake-message "continue"`; use `--no-wake` when no continuation
  is intended.

## Validation

Useful commands:

```bash
cargo test --workspace --all-targets
cd desktop/garyx-desktop && npm run build:ui
cd desktop/garyx-desktop && npm run test:smoke
```

For mobile Swift validation, run SwiftPM tests from the mobile package and
build the app target against the iOS simulator SDK. If the scheme-level
simulator build fails before compilation because Xcode cannot resolve an
eligible destination, use the target-level build to validate the same app
target:

```bash
cd mobile/garyx-mobile && swift test
cd mobile/garyx-mobile && xcodebuild -project GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator -configuration Debug build
```

When a packaged app is requested, or when validating packaging, install, release,
or startup behavior, run the packaging flow and launch the installed app:

```bash
cd desktop/garyx-desktop && npm run dist:dir
open -a Garyx
```

For narrower Rust checks, run the package-level target that matches the edit,
for example:

```bash
cargo test -p garyx-gateway --lib
cargo test -p garyx-router --all-targets
cargo test -p garyx-channels --lib
```

## Keep AGENTS.md and CLAUDE.md in sync

The repo-level `AGENTS.md` and `CLAUDE.md` are intentionally identical so that
every coding agent — regardless of which entry file it reads — gets the same
guidance. When you change one, change the other in the same commit. `AGENTS.md`
is the authoritative source; `CLAUDE.md` is the mirror copy.
