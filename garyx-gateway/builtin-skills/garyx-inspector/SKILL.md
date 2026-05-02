---
name: garyx-inspector
description: Use when Garyx/Codex needs to inspect why a bot or thread did not respond, responded incorrectly, or needs runtime diagnostics from Garyx. Prefer this skill when the user can provide a `thread_id` or `bot_id`, or when you need to inspect the current bot binding, a thread runtime state, lifecycle records, terminal reasons, bindings, or transcript paths through the Garyx CLI.
---

# Garyx Inspector

Use this skill to inspect Garyx runtime diagnostics from the terminal.

## Primary Entry Points

- By thread:
  `garyx thread history <thread_id>`
- By bot:
  `garyx bot status <bot_id>`
- Repo-local helper script:
  `garyx-gateway/builtin-skills/garyx-inspector/scripts/gary-thread-history.sh <thread_id>`

Examples:

- `garyx thread history thread::0afde5d5-e577-458b-9d49-ceae16ea97a1`
- `garyx bot status telegram:main`
- `garyx bot status telegram:main --json`

## Workflow

1. If the user already has a `thread_id`, start with `garyx thread history <thread_id>`.
2. If the user only knows the bot, start with `garyx bot status <bot_id>` to identify the current bound thread.
3. Read the lifecycle summary first:
   - `filtered`
   - `thread_resolved`
   - `run_started`
   - `reply_sent`
   - `reply_failed`
   - `run_interrupted`
4. If a failure reason is present, report it explicitly:
   - `policy_filtered`
   - `routing_rejected`
   - `reply_dispatch_failed`
   - `self_restart`
5. Only fall back to raw logs after the CLI output is insufficient.

## Rules

- Prefer `thread_id` as the primary inspection key.
- Prefer `bot_id` only when you need the bot's current main endpoint and bound thread.
- Use `--json` when you need to inspect full payloads or pass results into another tool.
- Do not claim a message was never received unless the diagnostics actually show no ingress/lifecycle record.
