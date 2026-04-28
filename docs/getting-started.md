# Getting Started

Garyx ships as a single CLI binary plus an optional desktop app. The
recommended path on macOS is the [Homebrew tap](https://github.com/Pyiner/homebrew-garyx);
Linux works through the same tap. From-source builds use cargo.

## Install

::: code-group

```bash [Homebrew]
brew tap pyiner/garyx
brew install pyiner/garyx/garyx
```

```bash [Shell script]
curl -fsSL https://raw.githubusercontent.com/Pyiner/garyx/main/install.sh | bash
```

```bash [From source]
git clone https://github.com/Pyiner/garyx
cd garyx
cargo build --release
# binary: target/release/garyx
```

:::

## 1. Initialize the config

`garyx.json` lives at `~/.garyx/garyx.json`. Generate a minimal one with:

```bash
garyx onboard
```

That seeds an `api` channel account, default agents, and the gateway block.
Or hand-write `~/.garyx/garyx.json`:

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

::: tip
Strings support `${VAR}` and `${VAR:-default}` env-var expansion at load
time, so secrets can stay out of the file.
:::

## 2. Start the gateway

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

## 3. Verify

```bash
curl -s http://127.0.0.1:31337/health
garyx status
garyx doctor
```

End-to-end through a fresh thread:

```bash
TID=$(garyx thread create --workspace-dir "$PWD" --json | jq -r .thread_id)
garyx thread send "$TID" "What does this workspace do?"
```

## 4. Add your first channel bot

Pick the platform you want Garyx to talk to.

### Telegram

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

### Feishu / Lark

Run the device-flow login (auto-fetches App ID / Secret):

```bash
garyx channels login feishu --domain feishu   # use --domain lark for 海外
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

The gateway opens a Feishu WebSocket; @-mention the bot in any chat it has been
added to.

### WeChat (企业微信 / 个人号 via ilinkai)

```bash
garyx channels login weixin
garyx gateway restart
```

The login flow scans a QR code in your terminal and writes the resulting
`token` and `uin` back into `~/.garyx/garyx.json`.

### What the resulting config looks like

After adding the Telegram example above, the relevant slice of `garyx.json`:

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

## What next

- [Configuration reference](/configuration) — the full schema for channels,
  plugins, providers, MCP servers, automations, and desktop behavior.
- [Architecture: Command List Design](/architecture/command-list-design) — how
  channel-native commands and shortcuts coexist.
