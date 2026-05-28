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
- Commit completed code changes before handoff. Stage only files changed for
  the current task.
- When committing from an agent, use the repository's configured Git author and
  committer metadata. Do not override local Git identity.
- When you create review tasks for your own work, notify `current-thread` so
  review results return to the task thread instead of a personal bot channel.

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
- `garyx-bridge`: provider orchestration for Claude Code, Codex, Gemini, and
  teams.
- `garyx-channels`: built-in channel runtimes and subprocess plugin host.
- `desktop/garyx-desktop`: Electron desktop app and shared renderer UI.
- `mobile/garyx-mobile`: iOS app, widget, and `GaryxMobileCore` Swift package.

## Product And Data Contracts

- Config lives in `~/.garyx/garyx.json`; gateway/router state is the source of
  truth for persisted runtime data.
- Thread records and endpoint state belong to `garyx-router`. Recent-thread
  projections must be updated at write time, not repaired by read routes.
- Workspace identity is always the absolute directory path string. Do not add
  workspace IDs or infer root workspace rows from thread metadata.
- Desktop and mobile root workspace lists come from gateway `/api/workspaces`
  application state.
- Configured bot account `config` is ordinary application state; do not add
  token-specific merge, redaction, or preservation paths beyond keeping real
  secrets out of committed fixtures.
- Provider, agent, team, bot, and channel identity presentation should resolve
  through shared presentation helpers instead of local view switch tables.

Detailed data and runtime contracts: @docs/agents/repository-contracts.md and
@docs/agents/workspace-paths.md.

## UI Direction

- The Mac app is the source of truth for information architecture, labels,
  field meaning, icon semantics, and Gateway-backed data models.
- Mobile may adapt layout and interaction for iOS, but must not invent new
  top-level concepts.
- Use native platform patterns: Electron/shadcn-style desktop surfaces where
  appropriate, and native grouped iOS management surfaces on mobile.
- Mobile route state, presentation mapping, formatting, and business-rule
  transformations should live in `GaryxMobileCore` with SwiftPM tests.
- Keep mobile SwiftUI feature surfaces in feature-specific files.

Detailed UI rules: @docs/agents/mobile-ui.md and @docs/agents/desktop-ui.md.

## Release And Runtime Boundaries

- Gateway code changes do not affect the running gateway until the binary is
  built, installed, and the managed gateway is restarted.
- iOS TestFlight releases are independent from macOS/gateway release flow. Do
  not trigger TestFlight unless the user explicitly asks in the current turn.
- Do not wire iOS uploads into version-tag release jobs unless the user
  explicitly asks for that coupling.

Detailed runtime, SDK, and workflow rules:
@docs/agents/gateway-runtime.md, @docs/agents/claude-sdk.md, and
@docs/agents/working-loop.md.

## Validation

Use the narrowest reliable validation for the touched area. Common commands and
fallbacks live in @docs/agents/validation.md.

## Keep AGENTS.md And CLAUDE.md In Sync

The repo-level `AGENTS.md` and `CLAUDE.md` are intentionally identical so every
coding agent gets the same guidance. When changing one, change the other in the
same commit. `AGENTS.md` is the authoritative source; `CLAUDE.md` is the mirror
copy.
