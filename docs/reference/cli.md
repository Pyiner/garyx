# CLI commands

Every `garyx` subcommand grouped by what you actually do with it. Run any
command with `--help` for the full flag list and arg descriptions.

## Setup

| Command | Use it for |
| --- | --- |
| `garyx onboard` | Guided first-time setup that writes a working `garyx.json`. |
| `garyx config init` | Write a defaults-only `garyx.json` without prompts. |
| `garyx config show` | Print the loaded, merged config (pretty JSON). |
| `garyx config get <path>` | Read a value by dotted path, e.g. `gateway.port`. |
| `garyx config set <path> <value>` | Update a value by dotted path. |
| `garyx config unset <path>` | Remove a key by dotted path. |
| `garyx config validate` | Validate the config file against the schema. |
| `garyx config path` | Print the absolute config file path. |

## Gateway

| Command | Use it for |
| --- | --- |
| `garyx gateway run` | Run the gateway in the foreground (blocks). |
| `garyx gateway install` | Register the managed service (launchd/systemd) and start it. Safe to re-run. |
| `garyx gateway start` / `stop` / `restart` | Control the managed service. `restart` requires `--wake ...` or `--no-wake`. |
| `garyx gateway uninstall` | Remove the managed unit / plist file. |
| `garyx gateway reload-config` | Reload config without restart. |
| `garyx gateway token` | Ensure a gateway auth token exists; print it. |

`gateway restart` and `/api/restart` restart the installed binary. They do not
build from source; install or copy a new binary first when you need code changes
to take effect.

`garyx gateway restart --wake <thread|task|bot> <target> --wake-message "..."`
restarts the managed gateway and, after the service is healthy again, sends the
wake message through the same target resolution used by `garyx thread send`.
The wake target is the only routing input; workspace is resolved from the target
thread/task/bot binding inside the gateway.

Bare `garyx gateway restart` is blocked because it can interrupt an active
streaming thread without waking it again. Use `--wake` for normal restarts, or
`garyx gateway restart --no-wake` when you intentionally want only a restart.

See [Service manager](/reference/service-manager) for what `install` actually
writes to disk.

## Channels {#channels}

| Command | Use it for |
| --- | --- |
| `garyx channels list` | List configured channel accounts. |
| `garyx channels add <channel> <account_id>` | Add a new account (Telegram, Feishu, WeChat, plugin id). |
| `garyx channels enable <channel> <account_id>` | Enable / disable an existing account. |
| `garyx channels remove <channel> <account_id>` | Delete an account from config. |
| `garyx channels login <channel>` | Channel-specific login flow (device-flow Feishu, QR-code WeChat, etc.). |

Common flags on `channels add`:

- `--token "<bot token>"` — Telegram
- `--app-id <id> --app-secret <secret> --domain feishu|lark` — Feishu / Lark
- `--uin <uin> --base-url <url>` — WeChat
- `--agent-id <id>` — bind the channel to a specific agent

## Plugins

| Command | Use it for |
| --- | --- |
| `garyx plugins install <path>` | Install a subprocess channel plugin from a binary. |
| `garyx plugins list` | List installed plugins. |
| `garyx plugins uninstall <id>` | Remove a plugin. |

## Threads

| Command | Use it for |
| --- | --- |
| `garyx thread list` | List threads (paginated). |
| `garyx thread get <thread_id>` | Fetch one thread record. |
| `garyx thread history <thread_id>` | Show thread history, tool calls, and runtime records. |
| `garyx thread create [--workspace-dir <path>] [--agent-id <id>] [--json]` | Create a new thread. |
| `garyx thread send thread <thread_id> [message]` | Send a message into a thread and stream the response. Reads stdin when `message` is omitted. |
| `garyx thread send task <task_ref> [message]` | Resolve a task to its backing thread, then send a message into that thread. |
| `garyx thread send bot <selector> [message]` | Resolve a bot's bound main thread inside the gateway, then send with that channel context. |

## Tasks

| Command | Use it for |
| --- | --- |
| `garyx task list --scope <channel/account>` | List tasks in a channel/account scope. |
| `garyx task create <scope> [--title <title>] [--body <body>] [--assignee <principal>] [--agent-id <id>] [--workspace-dir <path>] [--start]` | Create a task thread, optionally binding runtime agent/workspace and starting it. |
| `garyx task get <task_ref>` | Fetch one task. |
| `garyx task promote <thread_id>` | Promote an existing thread into a task. |
| `garyx task update <task_ref> --status <status> [--note <note>]` | Move a task through its lifecycle. |
| `garyx task claim / release / assign / unassign` | Manage task ownership. |
| `garyx task set-title / reopen / history` | Rename, reopen, or inspect task history. |

## Agents and teams

| Command | Use it for |
| --- | --- |
| `garyx agent list / get / create / update / upsert / delete` | CRUD on custom agents. |
| `garyx team list / get / create / update / delete` | CRUD on agent teams. |
| `garyx shortcuts list / get / set / delete` | Manage prompt shortcuts (a.k.a. commands). |

## Diagnostics

| Command | Use it for |
| --- | --- |
| `garyx status` | Show running gateway + channel summary. |
| `garyx doctor` | Run health checks (CLIs found, ports open, config valid). |
| `garyx audit` | Local environment / config audit. |
| `garyx logs path` | Print the log file path. |
| `garyx logs tail [--lines N]` | Tail the gateway log. |
| `garyx logs clear` | Truncate the log file. |
| `garyx bot status <bot_selector>` | Current bot main endpoint and bound thread status. |

## Updates

| Command | Use it for |
| --- | --- |
| `garyx update` | Download the latest release binary from GitHub and replace the running one. |

## Misc

| Command | Use it for |
| --- | --- |
| `garyx message --bot <selector> [text]` | Send an outbound channel message via a bot (e.g. `--bot telegram:main`); this does not start an agent run. |
| `garyx auto-research create / list / get / stop / patch / feedback / reverify` | Drive the auto-research loop. |
| `garyx wiki init / list / get / status / delete` | Manage wiki knowledge bases. |
| `garyx migrate thread-transcripts` {#migrate} | Move inline thread messages into transcript files. |

## Where to go next

- [Configuration](/configuration) — every dotted path you can pass to `config get/set`
- [Service manager](/reference/service-manager) — under-the-hood for `gateway install`
