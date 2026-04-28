---
name: setup
description: First-time Garyx setup — configure a Telegram bot and wire it to Claude Code or Codex. Run this when a new user clones the repo and wants to get Garyx running.
---

# Garyx Setup

Guide the user through first-time Garyx configuration. Two steps only — keep it fast.

## Prerequisites

Make sure the `garyx` CLI is available. Check with:

```bash
which garyx
```

If not found, build and install it:

```bash
cargo install --path garyx --bin garyx
```

## Step 1: Telegram Bot

Ask the user for:

1. **Bot token** (required) — the token from @BotFather, format: `123456789:AAHdqTcvCH1v...`
2. **Account name** (optional) — a short label for this bot. Default: `"main"`

If the user doesn't have a token yet, tell them:

> Open Telegram, search for @BotFather, send `/newbot`, follow the prompts, and paste the token here.

Once you have the token, run:

```bash
garyx onboard --force
garyx channels add telegram <account_name> --token "<token>"
```

Verify it was added:

```bash
garyx channels list
```

## Step 2: AI Provider

Check which providers are installed:

```bash
# Check Claude Code
which claude && claude --version 2>/dev/null

# Check Codex
which codex && codex --version 2>/dev/null
```

### If Claude Code is found

The default provider is already `claude_code` — no extra config needed. Tell the user:

> Claude Code detected. Your bot will use Claude Code as the AI provider (default).

### If only Codex is found

Update the bot's default thread agent to `codex`. Edit `~/.garyx/garyx.json` and set the Telegram account's `agent_id`:

```json
"agent_id": "codex"
```

Tell the user:

> Codex detected. Your bot is configured to use Codex as the AI provider.

### If neither is found

**Do NOT attempt to install anything.** Tell the user:

> Neither Claude Code nor Codex was found on this machine.
> You need at least one to run Garyx. Please install one:
>
> - **Claude Code**: `npm install -g @anthropic-ai/claude-code`
> - **Codex**: Download from https://openai.com/index/codex/
>
> After installing, run `/setup` again.

Stop here. Do not proceed.

## Step 3: Start

If both steps passed, start the gateway:

```bash
garyx gateway start
```

Verify it's running:

```bash
curl -s http://127.0.0.1:31337/health | head -1
```

Tell the user:

> Garyx is running! Send a message to your Telegram bot to test it.

## Rules

- **Never skip the token** — it's the only hard requirement.
- **Never install Claude Code or Codex for the user** — just detect and inform.
- **Keep it to two questions max** — token and account name.
- If `~/.garyx/garyx.json` already exists with a Telegram account, ask if they want to overwrite or add a new one.
