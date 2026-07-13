# Unified Command List Design

This design captures the Hermes-inspired command work. The public concept is a
command list.

## Command Kinds

Garyx currently has exactly two interaction command kinds:

- `channel_native`: built-in channel-only management commands, such as
  `/newthread`, `/threads [page|next|prev]`, and `/bindthread <n>`.
- `shortcut`: global prompt shortcuts configured by the user, such as
  `/summary -> "Summarize the active thread"`.

There is no per-user command model. There are no skill/plugin command kinds in
this command list.

## API

Use the shortcut management API:

- `GET /api/commands/shortcuts`
- `POST /api/commands/shortcuts`
- `PUT /api/commands/shortcuts/{name}`
- `DELETE /api/commands/shortcuts/{name}`

The internal command catalog and plugin RPC response use this shape:

```json
{
  "version": 1,
  "revision": "v1:...",
  "commands": [
    {
      "id": "builtin.router.threads",
      "name": "threads",
      "slash": "/threads",
      "aliases": [],
      "description": "Browse recent threads",
      "category": "Thread",
      "kind": "channel_native",
      "source": "builtin",
      "surfaces": ["plugin", "telegram"],
      "dispatch": { "type": "router_native", "key": "router.native.threads" },
      "args_hint": "[page|next|prev]",
      "visibility": "visible",
      "warnings": []
    }
  ],
  "warnings": []
}
```

## Visibility Rules

- Mac app/API/default views do not show `channel_native`.
- Channel/plugin views can request `channel_native` through `surface=plugin`
  plus a concrete `channel`.
- Telegram requests `surface=telegram&channel=telegram` and receives both
  `channel_native` and `shortcut` commands.
- Shortcuts are globally visible wherever shortcut execution is supported.

The visible thread-management menu is `newthread`, `threads`, and
`bindthread`. `threadprev` and `threadnext` remain reserved and recognizable
for one compatibility release, but are hidden from normal command lists and
Telegram menus. They return migration guidance instead of switching threads:
use `/threads prev|next`, then `/bindthread <n>`. Callers requesting
`include_hidden=true` may inspect those compatibility entries.

## Thread Command Semantics

- `/threads`, `/threads <page>`, `/threads next`, and `/threads prev` browse
  the global recent non-task thread projection in pages of 10. Addressed forms
  such as `/threads@bot_name 2` preserve the same arguments.
- `/bindthread <n>` switches to an absolute row number from pages successfully
  shown to that endpoint. A canonical `thread::<id>` may also be supplied for
  compatibility; either form is revalidated before binding.
- `/newthread` creates and binds a fresh thread, then clears the endpoint's
  accumulated recent-list snapshot.

`args_hint` is `[page|next|prev]` for `threads` and `<n>` for `bindthread`.
Invalid arguments are handled locally and never become model prompts.

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

The response includes the stable `revision` field. Plugins use `revision` to
avoid unnecessary remote menu sync calls.

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

- Payload uses `revision`.
- Plugin RPC uses `commands/list` and returns the command-list shape.
- Telegram sync includes both `channel_native` and `shortcut` commands.
- Telegram's visible native menu contains `newthread`, `threads`, and
  `bindthread`; deprecated navigation commands remain hidden.
- Mac app settings manage only prompt shortcuts.
