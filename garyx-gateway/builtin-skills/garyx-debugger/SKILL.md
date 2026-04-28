---
name: garyx-debugger
description: Use when Garyx/Codex needs to investigate why a bot or thread did not respond, responded incorrectly, or needs runtime diagnostics from Garyx. Prefer this skill when the user can provide a `thread_id` or `bot_id`, or when you need to inspect recent problem threads, lifecycle records, terminal reasons, bindings, or transcript paths through the `garyx debug` CLI.
---

# Garyx Debugger

Use this skill to inspect Garyx runtime diagnostics from the terminal.

## Primary Entry Points

- By thread:
  `garyx debug thread <thread_id>`
- By bot:
  `garyx debug bot <bot_id>`
- Repo-local helper script:
  `garyx-gateway/builtin-skills/garyx-debugger/scripts/gary-debug.sh thread <thread_id>`

Examples:

- `garyx debug thread thread::0afde5d5-e577-458b-9d49-ceae16ea97a1`
- `garyx debug bot telegram:main`
- `garyx debug bot telegram:main --json`

## Workflow

1. If the user already has a `thread_id`, start with `thread`.
2. If the user only knows the bot, start with `bot` to identify recent problem threads.
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

- Prefer `thread_id` as the primary debug key.
- Prefer `bot_id` when you need a summary of recent failures or problem threads.
- Use `--json` when you need to inspect full payloads or pass results into another tool.
- Do not claim a message was never received unless the diagnostics actually show no ingress/lifecycle record.
