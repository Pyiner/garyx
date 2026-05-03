# Channels

A **channel** is a transport that carries messages between humans (or other
bots) and Garyx. Built-in channels for Telegram, Feishu / Lark, WeChat, and
an HTTP / WebSocket `api` channel ship with the binary; you can install
additional channels as subprocess plugins.

## Built-in vs plugin channels

| Channel | Type | Notes |
| --- | --- | --- |
| `telegram` | built-in | Long-poll Bot API; multiple accounts supported. |
| `feishu` | built-in | WebSocket; supports `--domain feishu` and `--domain lark`. |
| `weixin` | built-in | Polling via [ilinkai](https://ilinkai.weixin.qq.com); QR-code login. |
| `api` | built-in | Local HTTP / WebSocket. Used by the desktop app, CLI, and MCP integrations. |
| `<plugin_id>` | plugin | Subprocess channels installed via `garyx plugins install`. |

Built-in and plugin channels share the **same** config shape and the same
account / binding model.

## Accounts

A channel can have multiple **accounts** — each one is an independent bot
identity. Accounts live under `channels.<channel_id>.accounts.<account_id>`
and have a fixed envelope plus a channel-specific `config` blob:

```json
{
  "channels": {
    "telegram": {
      "accounts": {
        "main": {
          "enabled": true,
          "name": "My Telegram Bot",
          "agent_id": "claude",
          "workspace_dir": "/path/to/garyx-work",
          "config": {
            "token": "${TELEGRAM_BOT_TOKEN}",
            "groups": { "-100123456789": { "enabled": true } }
          }
        }
      }
    }
  }
}
```

The envelope keys (`enabled`, `name`, `agent_id`, `workspace_dir`) are
identical across all channels. Everything else — bot tokens, app ids, group
allowlists, polling intervals — lives inside `config` and is validated by
the channel itself.

::: tip
You can manage accounts from the CLI without hand-editing the JSON. See
[`garyx channels add`](/reference/cli#channels) and
[`garyx channels login`](/reference/cli#channels).
:::

## Endpoint bindings

An **endpoint** is one specific destination *inside* a channel — a Telegram
chat id, a Feishu chat id, a WeChat user id. Endpoints are not configured
ahead of time; they are discovered lazily when the first message arrives,
and immediately bound to a thread.

The binding lookup order:

1. If the endpoint is already bound to a thread, route the message there.
2. Otherwise, create a fresh thread inheriting the account defaults and
   bind the endpoint to it.

You can inspect, rebind, or detach bindings from the desktop app or via the
HTTP API (`/api/channel-bindings/{bind,detach}`).

## The `api` channel

The `api` channel is special in two ways:

1. It has no on-the-wire transport — it speaks plain HTTP / WebSocket on
   the gateway port.
2. Every other channel internally goes through the same dispatcher as
   `api`, so all features (transcripts, MCP, automations) work identically.

The `api` channel is also what the desktop app, the `garyx thread send`
CLI, and MCP tool callbacks all use.

## Where to go next

- [Your first bot](/first-bot) — concrete walkthroughs for Telegram, Feishu, WeChat
- [Configuration](/configuration) — the full per-channel schema
- [CLI commands → channels / plugins](/reference/cli#channels)
