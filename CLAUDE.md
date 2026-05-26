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
When committing from an agent, ensure Git author and committer metadata are
synthetic too; override any local real-name or real-email Git config with
placeholders such as `Test User <bot@example.com>`.

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
  labels, field meaning, icon semantics, and Gateway-backed data models. The
  mobile app may adapt layout and interaction for iOS, but it must not invent
  new top-level concepts or move settings-only surfaces into the primary
  sidebar.
- Mobile Garyx management pages should use native iOS patterns: grouped lists,
  compact rows, section headers, top navigation actions, segmented controls
  where a page has two peer categories, and row-level ellipsis actions for
  secondary actions.
- Mobile chrome should follow current Apple Liquid Glass direction where
  available: navigation bars, bottom bars, and transient icon controls should
  use the app's adaptive glass/material helpers, while list content stays
  readable and deferential. Toolbar icons should default to monochrome
  `primary` styling; do not turn general navigation/actions blue unless color
  communicates a specific state or destructive/accent action.
- Mobile generic selected states should use monochrome `primary` styling via
  shared selection components such as `GaryxSelectionCheckmark`. Do not use
  green/accent checkmarks for ordinary selection; reserve color for semantic
  success, running, destructive, or warning states.
- Mobile glass styling should use Garyx's shared adaptive glass components,
  backed by Apple's official Liquid Glass APIs when the active Xcode/iOS SDK
  supports them. Do not introduce third-party glass libraries; keep SDK-gated
  fallbacks only so local and CI builds remain reproducible until the build
  toolchain moves to the latest iOS SDK.
- The mobile thread page top bar is the visual standard for custom top chrome:
  use the same floating glass icon controls, spacing, and compact glass title
  treatment for sibling mobile pages unless a native system navigation bar owns
  that surface.
- Mobile custom top chrome titles should be single-line only. Do not render
  dynamic counts, subtitles, or secondary metadata as a second line inside the
  glass title capsule; put that information in page content when needed.
- Mobile page backgrounds should stay near-white and integrated rather than
  heavy grouped gray. Avoid strong gray-background / white-card contrast on
  management pages; content rows can sit directly on the page background and use
  subtle separators, spacing, and typography for structure. Reserve glass for
  floating controls and navigation chrome, not repeated content rows.
- Do not port desktop management cards or exposed action button bars into the
  mobile app. Actions such as edit, delete, enable, pause, run, and detach
  should usually live behind each row's ellipsis glass action menu unless they
  are the primary action for the current screen. Do not use item left-swipe
  actions; horizontal swipes are reserved for navigation/sidebar gestures.
- Agent and team rows should use the same avatar data as the Mac app when
  available, with compact circular fallbacks rather than generic list icons.
- Mobile agent target selectors should use the shared
  `GaryxAgentTargetPickerControl` and `GaryxAgentAvatarView` rather than local
  Menu/Picker rows with generic person icons.
- Mobile provider, agent, and channel identity labels/icons should resolve
  through `GaryxProviderPresentation` and `GaryxChannelIdentityPresentation` in
  `GaryxMobileCore`. Do not add local provider/channel switch tables in views,
  widgets, or settings.
- The iOS home-screen recent-threads widget should start directly with thread
  rows, not a title/header row. Put pinned threads first but render them exactly
  like other rows, use agent/team avatars as the leading image, and let the
  visible row count scale with the widget family. Do not put a `ScrollView`
  inside the widget — WidgetKit views are static snapshots, and iOS marks any
  scrollable area with a yellow + ban-symbol overlay. Show only as many rows
  as comfortably fit each family at a tappable row height; older threads stay
  reachable from the in-app sidebar, not from widget scrolling.
- Channel and bot rows should use gateway-provided channel icon data first.
  Built-in channels need local raster fallbacks on mobile, and plugin SVG icons
  should be rasterized to PNG before catalog delivery so mobile does not fall
  back to initials for known icons or spin up WebKit on hot list rows.
- Mobile sidebars should keep workspace navigation as a two-stage flow: list
  workspace folders first, then show the selected workspace's threads in that
  workspace detail layer. Do not dump every workspace's sessions inline at once.
- Pinned mobile threads are the root-sidebar exception: render pinned threads
  inline as a compact section rather than hiding them behind a drilldown row.
- Mobile root-sidebar thread rows should not expose left-swipe actions; reserve
  horizontal swipes for opening/closing the sidebar. Put pin/archive actions in
  the thread page menu. Show pinned state with a neutral slanted pin immediately
  after the thread title that can be tapped to unpin, and show running state with
  a compact trailing spinner using `run_state`/`active_run_id` from recent-thread
  summaries. Under each thread title, show the thread workspace rather than the
  latest message preview; do not duplicate workspace metadata on the row's right
  edge.
- While the mobile sidebar is visible, silently refresh recent threads and pins
  about every three seconds so running-state indicators stay current without
  showing inline loading rows.
- Mobile sidebar root content should not render inline loading rows or raw
  thread/session lists. Use one unobtrusive global loading indicator and keep
  conversation/thread lists inside bot or workspace drilldown layers.
- Mobile sidebar primary navigation should show Automation and, directly below
  it, Workspace & Bots. Tasks, Auto Research, Agents, and Skills belong under
  Settings.
- Mobile leading-edge gestures and the top-left header control must share the
  same navigation action. Thread detail and first-level pages opened directly
  from the sidebar may open the sidebar; deeper pages must go back to their
  immediate parent instead of opening the sidebar.
- Mobile workspace and bot browsing belongs visually under Automation as the
  `Workspace & Bots` sibling row in the sidebar, not inside the Automation page.
  Do not put workspace or bot drilldowns back into the root thread list.
- Mobile `Workspace & Bots` root lists must be source-of-truth lists, not
  inferred aggregates. The Bots root list must strictly match configured bot
  accounts from `/api/configured-bots`; endpoint-only/channel conversation data
  may populate child drilldowns for those configured bots but must never create
  root bot rows. The Workspace root list must show only user-saved workspaces;
  paths inferred from recent threads, automations, auto-research runs, endpoints,
  or temporary workspaces may be form suggestions or file-link resolution hints
  only, not root navigation entries.
- Mobile bot rows that have multiple bound/openable conversations should expose
  a drilldown list like workspace rows. Keep the primary bot tap opening the
  root/default thread when one exists, but do not hide child conversations just
  because their threads are absent from the recent-thread page or because they
  are direct-message conversations.
- Mobile Automation rows should be plain tappable rows without leading icons or
  left-swipe actions. Tapping an automation opens its edit/detail page; list-row
  secondary actions live behind the row's ellipsis glass action menu.
- Mobile Automation existing-thread selectors should open a large bottom sheet
  backed by recent threads with search. Do not use compact menu pickers for
  choosing a target thread.
- Mobile Automation schedule editing should use a user-facing repeat/date/time
  model (daily, weekdays, weekly, monthly, no-repeat) rather than making
  interval hours the primary control.
- Mobile chat keyboard handling should treat the message area and composer as
  one vertical stack: the keyboard shrinks/moves both together, the composer
  remains attached below the messages, and the first tap or drag in the message
  area dismisses the keyboard before message scrolling resumes.
- Mobile chat transcript rendering should derive display rows from a pure
  user-turn model that mirrors the Mac app: completed final assistant answers
  stay outside turn collapsibles, intermediate activity folds under the turn,
  and tool groups default collapsed unless the containing turn is expanded.
- Mobile thread history should mirror the Mac app's user-turn pagination:
  request the latest 10 user turns by default, keep older pages behind
  `before_index`, and prefetch those older pages as the user scrolls upward.
- Mobile chat live updates should mirror the Mac app's layered model: keep the
  global gateway stream for push events, and run a selected-thread history
  reconcile loop while the app is active so passive inbound messages still
  appear if a stream event is missed.
- Mobile recent-thread polling is also a transcript data trigger: when a visible
  summary transitions from active/running to inactive/completed, hydrate that
  thread's latest history into the mobile message cache immediately. Opening the
  thread detail should show the cached final result first, not wait for the
  detail view's initial history request.
- While a mobile thread is running, show the bottom Thinking indicator when no
  assistant/tool activity is present yet; do not synthesize an empty Working
  turn row for a trailing user-only turn. Pure assistant/reasoning text is not a
  tool step and should stay as a normal assistant message rather than opening a
  Working row. Running Working rows should appear only for real tool activity
  and show an elapsed timer immediately, using the user-turn timestamp when
  available and a view-mount fallback otherwise.
- Mobile transcript AI activity loaders should use Garyx's sweep/shimmer
  treatment, such as `GaryxShimmerText`, for thinking, running work, and active
  tool summaries. Do not add `ProgressView` spinners to transcript activity rows.
- Mobile management screens should not expose create/edit forms inline by
  default. Use top navigation actions to open full-screen add/edit flows, not
  compact sheets or always-visible forms, and keep the main page focused on
  current state and lists.
- Mobile form sheets should use the shared `GaryxFormSheet` and
  `GaryxFormGroupedSection` patterns. Create/edit flows should use the same
  top cancel/save chrome as Automation; reserve inline save buttons for nested
  file or document actions inside an editor, not whole-page form submission.
- Pure mobile route state, presentation mapping, formatting, and business-rule
  transformations should live under `mobile/garyx-mobile/Sources/GaryxMobileCore`
  with SwiftPM tests. Keep the app target focused on SwiftUI view composition,
  bindings, platform adapters, and side-effect orchestration.
- Keep mobile SwiftUI feature surfaces in feature-specific files rather than
  adding large view trees to `GaryxMobileViews.swift`; use small shared helpers
  in the main views file only when they are genuinely reused across surfaces.
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
  indicator visible until the run is done. Pure assistant/reasoning text should
  remain a normal assistant message and must not create a Working/Worked
  summary row; reserve those rows for real tool activity.
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
