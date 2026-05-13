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
6. Update [docs/configuration.md](docs/configuration.md) when user-facing configuration or behavior changes.
7. Commit every completed code change before handoff. Stage only the files changed
   for the current task, and leave unrelated user work untouched.

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
