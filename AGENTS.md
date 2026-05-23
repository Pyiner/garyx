# Garyx Agent Guide

This file is the short repo-level guide for coding agents.

## IMPORTANT: No real personal data in tests or commits

This is a public repository. NEVER use real personal data in test fixtures,
docs, code samples, or commit messages. That includes — but is not limited to —
real names, real Telegram/WeChat/Feishu chat IDs and user IDs, real bot IDs,
real email addresses, real phone numbers, real `/Users/<username>` paths, and
real tokens or secrets. Use clearly synthetic placeholders instead, e.g.
`Test User`, `1000000001`, `/Users/test`, `bot@example.com`, `${TOKEN}`.

Before staging a change, scan the diff for personal data and remove it. If a
test needs to reference an account, build it from a placeholder fixture, not
from a real chat captured during local debugging.

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
- Thread records and known endpoint state: `garyx-router`.
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
6. For CLI/runtime features that depend on the managed gateway, install the new
   binary and run a synthetic end-to-end CLI smoke test against the running
   gateway after unit tests pass.
7. Update [docs/configuration.md](docs/configuration.md) when user-facing configuration or behavior changes.
8. Commit every completed code change before handoff. Stage only the files changed
   for the current task, and leave unrelated user work untouched.
9. iOS TestFlight releases are independent from the macOS/gateway release flow:
   do not wire iOS uploads into version-tag release jobs unless the user
   explicitly asks for that coupling.

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
  labels, field meaning, icon semantics, and Gateway-backed data models. The
  mobile app may adapt layout and interaction for iOS, but it must not invent
  new top-level concepts or move settings-only surfaces into the primary
  sidebar.
- Mobile Garyx management pages should use native iOS patterns: grouped lists,
  compact rows, section headers, top navigation actions, segmented controls
  where a page has two peer categories, and left-swipe row actions for
  secondary actions.
- Mobile chrome should follow current Apple Liquid Glass direction where
  available: navigation bars, bottom bars, and transient icon controls should
  use the app's adaptive glass/material helpers, while list content stays
  readable and deferential. Toolbar icons should default to monochrome
  `primary` styling; do not turn general navigation/actions blue unless color
  communicates a specific state or destructive/accent action.
- The mobile thread page top bar is the visual standard for custom top chrome:
  use the same floating glass icon controls, spacing, and compact glass title
  treatment for sibling mobile pages unless a native system navigation bar owns
  that surface.
- Mobile page backgrounds should stay near-white and integrated rather than
  heavy grouped gray. Avoid strong gray-background / white-card contrast on
  management pages; content rows can sit directly on the page background and use
  subtle separators, spacing, and typography for structure. Reserve glass for
  floating controls and navigation chrome, not repeated content rows.
- Do not port desktop management cards or exposed action button bars into the
  mobile app. Actions such as edit, delete, enable, pause, run, and detach
  should usually live behind row swipe actions unless they are the primary
  action for the current screen.
- Agent and team rows should use the same avatar data as the Mac app when
  available, with compact circular fallbacks rather than generic list icons.
- Channel and bot rows should use gateway-provided channel icon data first.
  Built-in channels need local raster fallbacks on mobile, and plugin SVG icons
  should be rasterized to PNG before catalog delivery so mobile does not fall
  back to initials for known icons or spin up WebKit on hot list rows.
- Mobile sidebars should keep workspace navigation as a two-stage flow: list
  workspace folders first, then show the selected workspace's threads in that
  workspace detail layer. Do not dump every workspace's sessions inline at once.
- Pinned mobile threads are the root-sidebar exception: render pinned threads
  inline as a compact section rather than hiding them behind a drilldown row.
- Mobile sidebar root content should not render inline loading rows or raw
  thread/session lists. Use one unobtrusive global loading indicator and keep
  conversation/thread lists inside bot or workspace drilldown layers.
- Mobile sidebar primary navigation should keep only Automation at the root;
  Tasks, Auto Research, Agents, and Skills belong under Settings.
- Mobile chat keyboard handling should treat the message area and composer as
  one vertical stack: the keyboard shrinks/moves both together, the composer
  remains attached below the messages, and the first tap or drag in the message
  area dismisses the keyboard before message scrolling resumes.
- Mobile chat transcript rendering should derive display rows from a pure
  user-turn model that mirrors the Mac app: completed final assistant answers
  stay outside turn collapsibles, intermediate activity folds under the turn,
  and tool groups default collapsed unless the containing turn is expanded.
- Mobile chat live updates should mirror the Mac app's layered model: keep the
  global gateway stream for push events, and run a selected-thread history
  reconcile loop while the app is active so passive inbound messages still
  appear if a stream event is missed.
- While a mobile thread is running, show the bottom Thinking indicator when no
  assistant/tool activity is present yet; do not synthesize an empty Working
  turn row for a trailing user-only turn. Running Working rows should show an
  elapsed timer immediately, using the user-turn timestamp when available and a
  view-mount fallback otherwise.
- Mobile management screens should not expose create/edit forms inline by
  default. Use top navigation actions to open full-screen add/edit flows, not
  compact sheets or always-visible forms, and keep the main page focused on
  current state and lists.
- Mobile UI screenshots used for product review should be captured against the
  running local gateway by default. Debug snapshot fixtures are only for isolated
  layout checks and must be called out explicitly when used.

## Desktop Transcript And File Tree Rules

- Treat transcript history as user-turn based: one user message plus the
  following agent activity until the next user message. Pagination, prefetch,
  folding, and final-answer visibility should use that unit rather than raw
  provider messages or tool-call counts.
- In collapsed desktop transcript turns, keep each completed user turn's final
  assistant text visible. Collapse only intermediate assistant/tool activity;
  do not hide older turn answers because the current thread is still running.
- While a desktop thread is still running, do not treat the trailing assistant
  text as the final answer. Keep the active user-turn row and its React
  container stable as tool calls arrive so existing message bubbles do not
  remount or replay entry animations.
- If an assistant text segment has streamed but the desktop thread is still
  running and the tail is not an active tool group, keep a bottom "Thinking"
  indicator visible until the run is done.
- Desktop interruption controls must be gateway-backed. The local Mac app
  process may not own the active WebSocket for runs started elsewhere or after
  a reload; after trying any local active socket, call the gateway chat
  interrupt endpoint so the bridge can interrupt or abort the active thread run.
- The workspace file browser should read directories on demand. Do not pre-scan
  child directories just to decide whether to show expansion affordances,
  especially on macOS where probing protected folders can trigger privacy
  prompts.
- Agent selectors should show only the agent or team identity. Do not append
  provider names such as Claude, Codex, or Gemini to selector labels or details;
  provider metadata belongs in dedicated settings/details surfaces outside
  pickers.

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
- Restart through the Garyx CLI with a wake target when working in an agent
  thread, for example `garyx gateway restart --wake thread <thread_id>
  --wake-message "continue"`.

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
