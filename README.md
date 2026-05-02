# Garyx

Garyx is a local-first AI gateway for running provider-backed agents from one
runtime. It connects a CLI, HTTP/WebSocket API, MCP tools, channel accounts, and
a macOS desktop app to shared thread history.

## Install

### Homebrew (recommended on macOS / Linux)

```bash
brew tap pyiner/garyx
brew install pyiner/garyx/garyx
```

### Shell Script

```bash
curl -fsSL https://raw.githubusercontent.com/Pyiner/garyx/main/install.sh | bash
```

### From Source

```bash
git clone https://github.com/Pyiner/garyx
cd garyx
cargo build --release
```

The built binary is `target/release/garyx`.

## Getting Started

### 1. Initialize the config

`garyx.json` lives at `~/.garyx/garyx.json`. Generate a minimal one with:

```bash
garyx onboard
```

That seeds an `api` channel account, default agents, and the gateway block.
You can also hand-write `~/.garyx/garyx.json`:

```json
{
  "gateway": {
    "host": "127.0.0.1",
    "port": 31337
  },
  "channels": {
    "api": { "accounts": { "main": { "enabled": true } } }
  }
}
```

> Strings support `${VAR}` and `${VAR:-default}` env-var expansion at load
> time, so secrets can stay out of the file.

### 2. Start the gateway

For day-to-day use, install it as a managed background service:

```bash
garyx gateway install   # writes ~/Library/LaunchAgents/com.garyx.agent.plist (macOS)
                        #     or ~/.config/systemd/user/garyx.service (Linux)
                        # and starts it; safe to re-run after config changes
garyx gateway restart   # pick up new config
garyx gateway stop      # stop the managed service
```

For one-off testing, run it in the foreground:

```bash
garyx gateway run
```

Logs land in `~/.garyx/logs/{stdout,stderr}.log`.

### 3. Verify

```bash
curl -s http://127.0.0.1:31337/health
garyx status
garyx doctor
```

Send a message into a fresh thread end-to-end:

```bash
TID=$(garyx thread create --workspace-dir "$PWD" --json | jq -r .thread_id)
garyx thread send thread "$TID" "What does this workspace do?"
```

### 4. Add your first channel bot

Pick the platform you want Garyx to talk to:

#### Telegram

1. Talk to [@BotFather](https://t.me/BotFather), `/newbot`, copy the token.
2. Register the account with Garyx:

   ```bash
   garyx channels add telegram main \
     --token "123456:ABC-DEF…" \
     --agent-id claude
   garyx gateway restart
   ```

3. DM your bot. The gateway picks up updates via long-polling and routes them
   through the agent you bound.

#### Feishu / Lark

1. Run the device-flow login (auto-fetches App ID / Secret):

   ```bash
   garyx channels login feishu --domain feishu   # use --domain lark for海外
   garyx gateway restart
   ```

   Or, if you already have credentials:

   ```bash
   garyx channels add feishu gary \
     --app-id cli_xxxxxxxx \
     --app-secret xxxxxxxxxxxx \
     --domain feishu \
     --agent-id claude
   ```

2. The gateway opens a Feishu WebSocket and you can @-mention the bot in any
   chat it has been added to.

#### WeChat (企业微信 / 个人号 via ilinkai)

```bash
garyx channels login weixin
garyx gateway restart
```

The login flow scans a QR code in your terminal and writes the resulting
`token` and `uin` back into `~/.garyx/garyx.json`.

#### What the resulting config looks like

After adding the Telegram example above, the relevant slice of `garyx.json`
will look like this:

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

Built-in channels (`telegram`, `feishu`, `weixin`, `api`) sit directly under
`channels.<channel_id>`. Each account has an envelope (`enabled`, `name`,
`agent_id`, `workspace_dir`) plus a channel-specific `config` blob.

## Common Commands

```bash
garyx gateway install           # install + start the managed service
garyx gateway restart           # restart after editing the config
garyx gateway run               # run in foreground (testing)
garyx config show               # dump effective config
garyx doctor                    # local environment health check
garyx status                    # show running gateway summary
garyx channels list             # list configured channel accounts
garyx channels add telegram main --token "$TELEGRAM_BOT_TOKEN" --agent-id claude
garyx channels login feishu     # device-flow login, writes credentials
garyx plugins install ./path/to/garyx-plugin-acmechat
garyx logs tail                 # tail gateway logs
garyx update                    # download latest release binary
```

## Desktop App

The macOS desktop app lives in `desktop/garyx-desktop`. Pre-built `.dmg` and
`.zip` bundles are attached to every GitHub release; for local hacking:

```bash
cd desktop/garyx-desktop
npm install
npm run build:ui
npm run dist:dir
```

`npm run dist:dir` builds and installs `Garyx.app` into `/Applications`.

## Configuration

The main configuration file is `~/.garyx/garyx.json`.

Read [docs/configuration.md](docs/configuration.md) for the full configuration
reference: channels, plugins, providers, managed MCP servers, automations,
and desktop behavior.

## Development

Useful local checks:

```bash
cargo test --workspace --all-targets
cd desktop/garyx-desktop && npm run build:ui
cd desktop/garyx-desktop && npm run test:smoke
```

## Project Layout

```text
garyx/                  CLI binary and gateway runtime assembly
garyx-models/           Shared config, provider, and thread data types
garyx-core/             Pure routing, key, label, and slash-command logic
garyx-router/           Thread records, transcripts, endpoint routing
garyx-bridge/           Claude Code, Codex, Gemini, and team orchestration
garyx-channels/         Built-in channel runtimes and plugin host protocol
garyx-gateway/          HTTP API, MCP server, automations, skills, desktop API
desktop/garyx-desktop/  macOS Electron desktop app
```

## License

See [LICENSE](LICENSE).
