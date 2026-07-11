# Garyx

Garyx is a local-first AI agent gateway. It connects provider-backed agents
to the places where work arrives: Telegram, Feishu / Lark, WeChat, the CLI,
HTTP / WebSocket clients, MCP tools, scheduled automations, and the macOS
desktop app. Every entry point shares the same thread history, workspace
binding, provider sessions, and task state.

Use Garyx when you want an agent to keep working across chat apps, terminals,
desktop sessions, and API calls without moving the conversation into a hosted
control plane.

## Why Garyx

| Capability | What it means |
| --- | --- |
| Local-first gateway | The gateway runs on your machine or server; config and transcripts live under `~/.garyx/`. |
| Multi-channel agents | Built-in Telegram, Feishu / Lark, WeChat, and local API channels share one routing model. |
| Provider choice | Route threads to Claude Code, Codex, Gemini, or custom agent definitions without changing channel setup. |
| Persistent threads | A chat, desktop tab, task, or CLI send can resume the same thread and provider session. |
| Workspace-aware runs | Each thread records the directory the agent should operate in, with optional isolated Git worktrees. |
| MCP integration | Each run gets a scoped Garyx MCP endpoint plus any upstream MCP servers from your config. |
| Tasks and automations | Promote threads into reviewable tasks or schedule recurring prompts that deliver through Garyx. |
| Extensible channels | Install subprocess channel plugins without rebuilding the gateway. |

## Install Garyx

The shell installer downloads the latest release archive, verifies its
checksum, and installs `garyx` to `/usr/local/bin` by default, using `sudo` if
that directory is not writable. The installer does not initialize config, start
the gateway, or re-sign the release binary.

```bash
curl -fsSL https://raw.githubusercontent.com/Pyiner/garyx/main/install.sh | bash
garyx --version
```

Other install paths:

```bash
# Homebrew
brew tap pyiner/garyx
brew install pyiner/garyx/garyx

# From source
git clone https://github.com/Pyiner/garyx
cd garyx
scripts/install-local-cli.sh
```

Install and start the managed gateway service, then run onboarding against the
running gateway:

```bash
garyx gateway install
garyx onboard
garyx status
```

`garyx gateway install` writes the launchd plist on macOS or the systemd user
unit on Linux, then starts the gateway. Re-run it after upgrades or service
file changes.

## Connect a Telegram bot

The first concrete Garyx flow should be a real channel bot. Create a Telegram
bot with [@BotFather](https://t.me/BotFather), store the token in an
environment variable, then register that account with Garyx:

```bash
export TELEGRAM_BOT_TOKEN="TOKEN_FROM_BOTFATHER"
garyx channels add telegram main \
  --token "$TELEGRAM_BOT_TOKEN" \
  --agent-id claude
garyx gateway restart --no-wake
```

DM the bot. Garyx will create or resume a thread for that Telegram chat and
route messages to the agent bound by `--agent-id`.

The example above uses the built-in `claude` agent. Pick the provider you plan
to use and make sure its CLI is logged in before expecting the bot to answer:

```bash
claude auth login
codex login
gemini auth login
```

The same account model works for Feishu / Lark, WeChat, and channel plugins:

```bash
garyx channels login feishu --domain feishu
garyx channels login weixin
garyx plugins install ./path/to/garyx-plugin-example
```

## Common workflows

| Workflow | Command or surface |
| --- | --- |
| Chat with a workspace from the terminal | `garyx thread create --workspace-dir "$PWD"` then `garyx thread send ...` |
| Route a channel bot to an agent | `garyx channels add <channel> <account_id> --agent <agent_id>` |
| Continue a bot conversation from the CLI | `garyx thread history <thread_id>` and `garyx thread send thread <thread_id> ...` |
| Delegate work as a reviewable task | `garyx task create --title "..." --body "..." --agent <agent_id> --notify current-thread` |
| Schedule recurring agent work | `garyx automation create --label "Daily triage" --prompt "..." --every-hours 24` |
| Inspect gateway issues | `garyx status`, `garyx doctor`, `garyx logs tail` |
| Update the CLI | `garyx update` |
| Manage channel plugins | `garyx plugins list` and `garyx plugins install <path>` (plugins self-update) |
| Work on the mobile client | `cd mobile/garyx-mobile && swift test` |

## Architecture

```text
Humans and systems
  -> Telegram / Feishu / WeChat / CLI / Desktop / HTTP / WebSocket
  -> Garyx gateway
  -> Router, transcripts, endpoint bindings, tasks, automations
  -> Provider bridge: Claude Code / Codex / Gemini
  -> Scoped MCP endpoint and configured upstream MCP servers
```

Important runtime boundaries:

- The gateway is the single writer for channel endpoint bindings and task
  state.
- A thread's `workspace_dir` is fixed once the thread starts; create a new
  thread for a different execution directory.
- Code changes do not affect a managed gateway until the binary is installed
  and the service is restarted.
- The macOS app connects to the gateway API. It does not need a separate
  bundled web dashboard served by the gateway.

## Configuration

Garyx reads its main config from:

```text
~/.garyx/garyx.json
```

Keep secrets in environment variables and reference them from config:

```json
{
  "channels": {
    "telegram": {
      "accounts": {
        "main": {
          "enabled": true,
          "agent_id": "claude",
          "config": {
            "token": "${TELEGRAM_BOT_TOKEN}"
          }
        }
      }
    }
  }
}
```

Strings support `${VAR}` and `${VAR:-default}` expansion at load time.

Read the full configuration reference at
[docs/configuration.md](docs/configuration.md).

## Security and privacy

Garyx is local-first, but it is not a sandbox. Provider CLIs run with the
permissions of the gateway process, and a workspace directory is a development
context rather than a security boundary.

For public issues, tests, docs, and commits:

- Use placeholders such as `TOKEN_FROM_BOTFATHER`, `/path/to/repo`, and
  `thread::<id>`.
- Do not paste real chat ids, user ids, bot ids, email addresses, tokens,
  provider OAuth strings, or local personal paths.
- Prefer environment variables over literal secrets in `garyx.json`.
- Use `garyx logs tail` for diagnostics and redact provider or channel
  credentials before sharing logs.

See [docs/security.md](docs/security.md) for the longer checklist.

## Documentation

| Page | Covers |
| --- | --- |
| [Introduction](docs/index.md) | Product overview and quickstart |
| [Installation](docs/installation.md) | Install paths, gateway service, verification |
| [Your first bot](docs/first-bot.md) | Telegram, Feishu / Lark, and WeChat setup |
| [Threads & workspaces](docs/concepts/threads-and-workspaces.md) | Thread routing, workspace inheritance, provider sessions |
| [Channels](docs/concepts/channels.md) | Built-in channels, plugin channels, endpoint bindings |
| [Providers](docs/concepts/providers.md) | Claude Code, Codex, Gemini, auth, fallback behavior |
| [MCP integration](docs/concepts/mcp.md) | Garyx MCP tools and upstream MCP server config |
| [CLI commands](docs/reference/cli.md) | Every supported `garyx` command group |
| [Security](docs/security.md) | Secret handling, logs, local runtime boundaries |

## Desktop app

The macOS desktop app lives in `desktop/garyx-desktop`. Pre-built `.dmg` and
`.zip` bundles are attached to GitHub releases. For local development:

```bash
cd desktop/garyx-desktop
npm install
npm run build:ui
npm run dist:dir
```

`npm run dist:dir` builds and installs `Garyx.app` into `/Applications`.

## Mobile client

The mobile work-in-progress lives in `mobile/garyx-mobile`. It is the
Garyx-owned iOS client for the managed gateway.

## Development

Useful local checks:

```bash
cargo test --workspace --all-targets
cd desktop/garyx-desktop && npm run build:ui
cd desktop/garyx-desktop && npm run test:smoke
```

Project layout:

```text
garyx/                  CLI binary and gateway runtime assembly
garyx-models/           Shared config, provider, and thread data types
garyx-core/             Pure routing, key, label, and slash-command logic
garyx-router/           Thread records, transcripts, endpoint routing
garyx-bridge/           Claude Code, Codex, and Gemini orchestration
garyx-channels/         Built-in channel runtimes and plugin host protocol
garyx-gateway/          HTTP API, MCP server, automations, skills, desktop API
desktop/garyx-desktop/  macOS Electron desktop app
```

## License

See [LICENSE](LICENSE).
