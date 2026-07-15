# Your first bot

This page walks through getting the first channel bot answering messages.
Pick the platform you actually use; each section is self-contained.

::: tip Prerequisites
You should already have:

- `garyx --version` working ([Installation](/installation))
- the gateway running (`garyx status` returns OK)
- at least one provider logged in ([Providers](/concepts/providers))
:::

## Telegram

The fastest path. You only need a bot token from [@BotFather](https://t.me/BotFather).

1. Talk to [@BotFather](https://t.me/BotFather), `/newbot`, follow prompts,
   and store the token in your shell environment.
2. Register the account with Garyx and bind it to an agent:

   ```bash
   export TELEGRAM_BOT_TOKEN="TOKEN_FROM_BOTFATHER"
   garyx channels add telegram main \
     --token "$TELEGRAM_BOT_TOKEN" \
     --agent claude
   garyx gateway restart
   ```

3. DM your bot. Garyx pulls updates with long-polling and routes them through
   the agent you bound on `--agent`.

::: info Group chats
By default the bot will only listen in DMs. To enable a group, edit the
account in `~/.garyx/garyx.json` and add the chat under
`channels.telegram.accounts.<id>.config.groups`. See
[Configuration → Telegram](/configuration#telegram) for the schema.
:::

## Feishu / Lark

Use the device-flow login if you do not have an app yet:

```bash
garyx channels login feishu --domain feishu   # use --domain lark for 海外
garyx gateway restart
```

The login flow walks you through creating an app and writes the resulting
`app_id` / `app_secret` back into `garyx.json`.

If you already have an app, register it directly:

```bash
garyx channels add feishu gary \
  --app-id cli_xxxxxxxx \
  --app-secret xxxxxxxxxxxx \
  --domain feishu \
  --agent claude
```

The gateway opens a Feishu WebSocket; @-mention the bot in any chat it has
been added to.

## WeChat (企业微信 / 个人号 via ilinkai)

```bash
garyx channels login weixin
garyx gateway restart
```

The login flow scans a QR code in your terminal and writes the resulting
`token` and `uin` back into `garyx.json`.

## What the resulting config looks like

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
`agent_id`, `workspace_dir`) plus a channel-specific `config` blob — see
[Concepts → Channels](/concepts/channels) for why.

## Where to go next

- [Concepts → Channels](/concepts/channels) — accounts vs endpoint bindings,
  built-in vs plugin
- [Configuration](/configuration) — the full per-channel schema
- [CLI commands](/reference/cli) — every `garyx channels` subcommand
- [Security](/security) — what not to paste into docs, logs, or issues
