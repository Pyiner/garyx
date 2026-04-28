# Unified Command List Design

This design captures the Hermes-inspired command work. The public concept is a
command list.

## Command Kinds

Garyx currently has exactly two interaction command kinds:

- `channel_native`: built-in channel-only management commands, such as
  `/threads`, `/newthread`, `/threadprev`, `/threadnext`, and `/loop`.
- `shortcut`: global prompt shortcuts configured by the user, such as
  `/summary -> "Summarize the active thread"`.

There is no per-user command model. There are no skill/plugin command kinds in
this command list.

## API

Use the direct commands API:

- `GET /api/commands`
- `GET /api/commands/shortcuts`
- `POST /api/commands/shortcuts`
- `PUT /api/commands/shortcuts/{name}`
- `DELETE /api/commands/shortcuts/{name}`

`GET /api/commands` accepts filters:

- `surface=plugin&channel=telegram&account_id=main`
- `surface=telegram&channel=telegram&account_id=main`
- `surface=desktop_composer`
- `include_hidden=true`

Response shape:

```json
{
  "version": 1,
  "revision": "v1:...",
  "commands": [
    {
      "id": "builtin.router.newthread",
      "name": "newthread",
      "slash": "/newthread",
      "aliases": [],
      "description": "Start a new thread",
      "category": "Thread",
      "kind": "channel_native",
      "source": "builtin",
      "surfaces": ["plugin", "telegram"],
      "dispatch": { "type": "router_native", "key": "router.native.newthread" },
      "visibility": "visible",
      "warnings": []
    }
  ],
  "warnings": []
}
```

## Visibility Rules

- Default `GET /api/commands` returns shortcuts only.
- Mac app/API/default views do not show `channel_native`.
- Channel/plugin views can request `channel_native` through `surface=plugin`
  plus a concrete `channel`.
- Telegram requests `surface=telegram&channel=telegram` and receives both
  `channel_native` and `shortcut` commands.
- Shortcuts are globally visible wherever shortcut execution is supported.

## Plugin Contract

Channel plugins fetch the command list through host RPC:

```text
commands/list
```

Suggested params:

```json
{
  "account_id": "main",
  "surface": "telegram",
  "channel": "telegram",
  "include_hidden": false
}
```

The response is the same command-list shape as `GET /api/commands`, including
the stable `revision` field. Plugins use `revision` to avoid unnecessary remote
menu sync calls.

## Telegram Sync

Telegram owns BotCommands publishing:

- It projects command-list entries into Telegram BotCommand payloads.
- It syncs immediately on channel startup.
- It retries every 10 minutes.
- It skips remote calls when the projected command list has not changed.

Gateway core does not expose a manual Telegram command-sync endpoint.

## Validation Rules

- Shortcut names are normalized without leading `/`.
- Shortcut names must match `[a-z0-9_]` and be at most 32 chars.
- Shortcuts must map to prompt text.
- Shortcuts cannot collide with `channel_native` command names.
- Persisted invalid shortcut entries are omitted from command output and
  reported as warnings.

## Acceptance Criteria

- Command API uses `/api/commands`.
- Payload uses `revision`.
- Plugin RPC uses `commands/list` and returns the command-list shape.
- Telegram sync includes both `channel_native` and `shortcut` commands.
- Mac app settings manage only prompt shortcuts.
