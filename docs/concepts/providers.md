# Providers

A **provider** is the thing that actually executes a model run on behalf of
an agent. Garyx ships with three:

| Provider key | Backed by | Auth model |
| --- | --- | --- |
| `claude_code` | [Claude Code CLI](https://github.com/anthropics/claude-code) | OAuth long-lived token via `claude setup-token` (recommended) or interactive `claude auth login`. |
| `codex_app_server` | [Codex CLI](https://github.com/openai/codex) app-server | OpenAI account login via `codex login`. |
| `gemini_cli` | [Gemini CLI](https://github.com/google-gemini/gemini-cli) | Google account login via `gemini auth login`. |

Providers are not pinned per agent — Garyx auto-detects which CLIs are
installed at startup and registers them as `claude_code`, `codex_app_server`,
and `gemini_cli` respectively.

## How runs find a provider

When a message lands on a thread:

1. Look up the thread's agent (`agent_id`).
2. Resolve the agent's provider preference: each agent has a default
   provider, with optional fallbacks.
3. Spawn the provider CLI in stream-json mode, send an `initialize` control
   request, then stream user content.
4. Stream `assistant_delta` events back to the channel that triggered the
   run.

Resume tokens (Claude Code / Codex SDK session ids) are kept per-thread, so
a single Telegram chat preserves context across many runs without you
managing that state explicitly.

## Authenticating Claude Code

This is the path most users take. Two recommended modes:

### Long-lived OAuth token (best for headless / launchd)

```bash
claude setup-token   # opens a browser, returns sk-ant-oat01-…
echo 'export CLAUDE_CODE_OAUTH_TOKEN="sk-ant-oat01-…"' >> ~/.zshrc
chmod 600 ~/.zshrc
```

The Garyx launchd plist (and the systemd unit on Linux) re-enters your
login shell with `-lic`, so anything you `export` in `~/.zshrc` reaches the
gateway and from there the spawned `claude` CLI.

### Interactive login (best for desktop sessions)

```bash
claude auth login
```

This stores credentials in your macOS Keychain. Works while you are logged
into the GUI session; on Macs that primarily run headless we recommend the
long-lived token instead, because Keychain access can be locked from
launchd-spawned processes.

## Authenticating Codex / Gemini

```bash
codex login           # OpenAI account
gemini auth login     # Google account
```

Each CLI persists its own credentials. As long as the CLI binary is on the
gateway's `PATH`, Garyx will pick it up automatically.

## What happens when a token expires

If a provider call fails with an auth error or the SDK times out on
`initialize`, Garyx logs a warning and surfaces a clear error event into
the thread. Recover by re-running the provider's login command — no Garyx
restart is required, the next run will pick up fresh credentials.

::: tip Diagnosing silently-stuck providers
If a provider hangs (e.g. claude prompts for `/login` interactively but
nothing on stdin to receive it), the gateway logs:

```
WARN claude_provider: failed to connect to claude: Timeout: Control request timed out
```

That string is the smoking gun for "your provider CLI is logged out".
:::

## Where to go next

- [Installation](/installation) — recommended order: gateway, then provider auth
- [MCP integration](/concepts/mcp) — what tools the agent gets back through Garyx
- [Configuration](/configuration) — picking a default and fallback provider
