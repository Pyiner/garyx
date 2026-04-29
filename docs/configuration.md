# Garyx Configuration

Garyx reads its main configuration from:

```text
~/.garyx/garyx.json
```

The file is JSON. Values can reference environment variables with
`${NAME}` or `${NAME:default}`.

## Minimal Config

```json
{
  "gateway": {
    "host": "127.0.0.1",
    "port": 31337
  },
  "channels": {
    "api": {
      "accounts": {}
    }
  }
}
```

Start the gateway with:

```bash
garyx gateway run
```

Check it with:

```bash
curl http://127.0.0.1:31337/health
```

## Gateway

```json
{
  "gateway": {
    "host": "127.0.0.1",
    "port": 31337,
    "public_url": "",
    "auth_token": ""
  }
}
```

Fields:

| Field | Default | Description |
| --- | --- | --- |
| `host` | `127.0.0.1` | Address the gateway binds to. |
| `port` | `31337` | HTTP, WebSocket, and MCP port. |
| `public_url` | `""` | Optional public URL used in channel message links. |
| `auth_token` | `""` | Required bearer token for all protected gateway APIs. Create one on the gateway host with `garyx gateway token`; `/health` remains public. |

## Channels

Garyx stores user-facing channel accounts directly under `channels.<channel_id>`.
Built-in channels and external subprocess plugins use the same account shape.

```json
{
  "channels": {
    "telegram": {
      "accounts": {
        "main": {
          "enabled": true,
          "name": "Telegram",
          "agent_id": "claude",
          "workspace_dir": "/path/to/workspace",
          "config": {
            "token": "${TELEGRAM_BOT_TOKEN}"
          }
        }
      }
    }
  }
}
```

Common account fields:

| Field | Description |
| --- | --- |
| `enabled` | Whether the account should run. |
| `name` | Optional display name in the desktop app. |
| `agent_id` | Agent or team used for new inbound threads. |
| `workspace_dir` | Default workspace for new threads from this account. |
| `config` | Channel-specific fields declared by the built-in channel or plugin. |

The desktop Add Bot flow validates account connectivity through the gateway
before writing the account. Telegram verifies the bot token with `getMe`;
Feishu/Lark verifies app credentials by requesting a tenant access token.
Channels without a safe probe explicitly report the validation as skipped.

### Telegram

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

### Feishu / Lark

```json
{
  "channels": {
    "feishu": {
      "accounts": {
        "main": {
          "enabled": true,
          "agent_id": "claude",
          "config": {
            "app_id": "${FEISHU_APP_ID}",
            "app_secret": "${FEISHU_APP_SECRET}",
            "domain": "feishu",
            "require_mention": true,
            "topic_session_mode": "disabled"
          }
        }
      }
    }
  }
}
```

`domain` selects Feishu (`"feishu"`) or Lark (`"lark"`).
Direct messages and group chats are accepted by default. `require_mention`
controls whether group messages need to mention the bot before dispatch, and
`topic_session_mode` controls whether a group uses one shared session
(`"disabled"`) or splits sessions by Feishu topic/thread (`"enabled"`).

You can also use the interactive login flow:

```bash
garyx channels login feishu --account main
```

To refresh an existing account's credentials while preserving its display
name, workspace, agent binding, and plugin-specific config, pass that account
id to `--reauthorize`:

```bash
garyx channels login feishu --reauthorize main
```

### Weixin

```json
{
  "channels": {
    "weixin": {
      "accounts": {
        "main": {
          "enabled": true,
          "agent_id": "claude",
          "config": {
            "token": "${WEIXIN_BOT_TOKEN}",
            "uin": "${WEIXIN_UIN}"
          }
        }
      }
    }
  }
}
```

You can also use the interactive QR login flow:

```bash
garyx channels login weixin --account main
```

For QR reauthorization, omit `--account` so Garyx can use the bot id returned
by Weixin. If that id differs from the previous one, the previous account is
disabled by default; add `--forget-previous` to remove it after the new account
is saved:

```bash
garyx channels login weixin --reauthorize main --forget-previous
```

### External Plugins

Build or obtain a plugin binary, then install it:

```bash
garyx plugins install ./path/to/garyx-plugin-acmechat
garyx gateway restart
```

After installation, configure it like any built-in channel:

```json
{
  "channels": {
    "acmechat": {
      "accounts": {
        "main": {
          "enabled": true,
          "agent_id": "claude",
          "config": {
            "token": "${ACMECHAT_TOKEN}",
            "base_url": "https://chat.example.com"
          }
        }
      }
    }
  }
}
```

The plugin's JSON Schema is the UI-facing account model. When the gateway
serves `/api/channels/plugins`, each account config is projected through that
schema before it is returned, so stale or internal keys outside the schema do
not become editable fields.

If the plugin declares an auth flow, use:

```bash
garyx channels login acmechat --account main
```

The same reauthorization convention works for plugins with auth flows:

```bash
garyx channels login acmechat --reauthorize main
```

## Commands

Garyx exposes one command list with two command kinds:

- `channel_native`: built-in channel-only management commands, such as thread
  navigation. They are not editable and are only exposed to channel plugins.
- `shortcut`: global custom prompt shortcuts persisted in `commands`; each
  shortcut maps a slash name to prompt text.

```bash
curl 'http://127.0.0.1:31337/api/commands?surface=plugin&channel=telegram&account_id=main'
```

Shortcut commands are managed through:

```text
/api/commands/shortcuts
```

The same shortcuts can be managed from the CLI:

```bash
garyx commands list
garyx commands set summary --description "Summarize thread" --prompt "Summarize the current thread"
garyx commands get summary
garyx commands delete summary
```

Channel plugins fetch commands through the plugin RPC method `commands/list`.
Telegram owns its BotCommands publishing and refreshes the projected menu on
startup and every 10 minutes; the gateway no longer exposes a manual Telegram
command-sync endpoint.

## Agents and Teams

Each channel account can set `agent_id`.

Use a built-in provider agent:

```json
{ "agent_id": "claude" }
```

Use a custom agent or an agent team by setting the same `agent_id` used in your
Garyx agent/team configuration. The CLI account setup flow can also prompt for
an agent when `--agent-id` is omitted.

## Managed MCP Servers

Put managed MCP servers under `mcp_servers` in `~/.garyx/garyx.json`.

```json
{
  "mcp_servers": {
    "filesystem": {
      "enabled": true,
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    }
  }
}
```

Garyx syncs managed MCP configuration into the downstream provider config files
it controls.

## Desktop App

The desktop app connects to a gateway URL and stores its own local desktop
settings. It does not read gateway-local files directly.

Set the gateway URL and token in the desktop connection settings. Create or
print the token on the gateway host with:

```bash
garyx gateway token
```

If a protected gateway API returns an authorization error, the desktop app
returns to the Gateway Setup screen so you can paste a fresh token and continue.
Saving the gateway URL/token pair first verifies connectivity. Only verified
saves are written to local desktop state and added to the Gateway URL history;
the field can still be edited manually.

Gateway runtime settings, including `agent_defaults.heartbeat` defaults, are
edited from the Gateway tab in the desktop Settings view.
Thread history is persisted through transcript records; backend selection is no
longer exposed as a gateway setting.

Packaged macOS builds check for app updates automatically. You can also open
Settings > Mac App and use Check Now to manually refresh the update state.
Development builds report update checks as unavailable because there is no
signed app bundle to replace. Locally copied or ad-hoc signed `.app` bundles
also disable automatic updates; install a signed DMG once so macOS ShipIt can
validate future updates against the app's Developer ID signature.

The desktop language preference is stored in local desktop state. The default
is `system`, which follows macOS and falls back to English; users can also
choose English or Chinese explicitly from Settings.

## CLI Update

The CLI does not auto-update. Use:

```bash
garyx update
```

You can pin a version:

```bash
garyx update --version 0.1.7
```

## Useful Commands

```bash
garyx config show
garyx config validate
garyx channels list
garyx channels add telegram main --token "$TELEGRAM_BOT_TOKEN"
garyx channels login feishu --account main
garyx plugins list
garyx gateway restart
garyx logs tail
```

## Testing a Source Checkout

```bash
cargo test --workspace --all-targets
cd desktop/garyx-desktop && npm run build:ui
cd desktop/garyx-desktop && npm run test:smoke
```
