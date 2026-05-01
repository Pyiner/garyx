# Garyx Agent Guide

This file is the short repo-level guide for coding agents.

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

## Working Loop

1. Read the local code around the change before editing.
2. Keep the change scoped to the smallest correct surface.
3. Prefer existing crate and UI patterns over new abstractions.
4. Run focused deterministic tests for the touched area.
5. When touching the macOS app under `desktop/garyx-desktop`, package and launch
   the app before handoff so the installed desktop surface is verified, not only
   the dev build.
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
and interaction feedback. Before handoff, still run the packaged-app validation
flow below so the installed desktop surface is verified too.

## Validation

Useful commands:

```bash
cargo test --workspace --all-targets
cd desktop/garyx-desktop && npm run build:ui
cd desktop/garyx-desktop && npm run test:smoke
```

For macOS app changes, run the packaging flow and launch the installed app:

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
