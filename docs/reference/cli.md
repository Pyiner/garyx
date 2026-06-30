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

`garyx gateway restart --wake all` captures the threads that are running in
`recent_threads` before restart and sends each one `continue` after the new
gateway starts. Use `--wake-message "..."` to override the message.

Bare `garyx gateway restart` is blocked because it can interrupt an active
streaming thread without waking it again. Use `--wake` for normal restarts, or
`garyx gateway restart --no-wake` when you intentionally want only a restart.

See [Service manager](/reference/service-manager) for what `install` actually
writes to disk.

## Channels {#channels}

| Command | Use it for |
| --- | --- |
| `garyx channels list` | List configured channel accounts. |
| `garyx channels add <channel> <account_id>` | Add a new account (Telegram, Discord, Feishu, WeChat, plugin id). |
| `garyx channels enable <channel> <account_id>` | Enable an existing account. |
| `garyx channels disable <channel> <account_id>` | Disable an existing account. |
| `garyx channels remove <channel> <account_id>` | Delete an account from config. |
| `garyx channels login <channel>` | Channel-specific login flow (device-flow Feishu, QR-code WeChat, etc.). |

Common flags on `channels add`:

- `--token "<bot token>"` — Telegram or Discord
- `--app-id <id> --app-secret <secret> --domain feishu|lark` — Feishu / Lark
- `--uin <uin> --base-url <url>` — WeChat
- `--agent-id <id>` — bind the channel to a specific agent

## Commands and shortcuts

| Command | Use it for |
| --- | --- |
| `garyx commands list` | List command definitions for a surface such as router, desktop composer, Telegram, API chat, or plugin. |
| `garyx commands get <name>` | Show one prompt shortcut. |
| `garyx commands set <name> [--prompt <text>] [--description <text>]` | Create or update a global prompt shortcut. Reads stdin when `--prompt` is omitted. |
| `garyx commands delete <name>` | Delete a prompt shortcut. |

`garyx shortcuts` and `garyx shortcut` are aliases for the same command group.

## Plugins

| Command | Use it for |
| --- | --- |
| `garyx plugins install <path>` | Install a subprocess channel plugin from a binary. |
| `garyx plugins list` | List installed plugins. |
| `garyx plugins uninstall <id>` | Remove a plugin. |
| `garyx plugins update [<name>]` | Update one installed subprocess plugin (or all when `<name>` is omitted). Supports `--version`, `--from`, `--target`, `--check`, `--force`, `--json`. Built-in channels (`telegram`, `discord`, `feishu`, `weixin`) are rejected with a redirect to `garyx update`. |

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

## Dreams

| Command | Use it for |
| --- | --- |
| `garyx dream list [--since-hours N] [--json]` | List recent topic summaries with the thread spans they came from. |
| `garyx dream scan [--since-hours N] [--mode auto|claude|heuristic] [--json]` | Scan recent user messages, persist refreshed Dreams topics, and print the scan result. |
| `garyx dream auto [status|on|off] [--json]` | Show or set the hourly automatic Dreams scan switch. |
| `garyx dream show <dream_id> [--json]` | Show one persisted Dreams topic and its thread spans. |

`garyx dreams` is an alias for the same command group. Scans use the running
gateway and store their results in the local Garyx SQLite database.
The `dreams.enabled` gateway setting controls app entry points and automatic
scans; manual CLI scans still run when the switch is off. Manual scans upsert
topic changes incrementally instead of deleting older spans outside the
requested window.

## Tasks

| Command | Use it for |
| --- | --- |
| `garyx task list [--status <status>] [--limit <n>] [--offset <n>]` | List tasks, done tasks included. Filter with `--status todo\|in_progress\|in_review\|done`; page with `--limit` / `--offset`. |
| `garyx task create [--title <title>] [--body <body>] [--agent <id> \| --team <id> \| --workflow <id>] [--workspace-dir <path>] [--notify <target>]` | Create a task thread backed by an Agent, Agent Team, or Workflow executor; picking an executor starts the run immediately, otherwise the task is created as a `todo` placeholder. `--notify` defaults to the current thread (or `none` outside a thread); override with `current-thread`, `thread <thread_id>`, `bot <channel:account_id>`, or `none`. |
| `garyx task get <task_ref>` | Fetch one task. |
| `garyx task update <task_ref> --status <status> [--note <note>]` | Move a task through its lifecycle. Garyx moves an in-progress task to review when its agent run stops; only mark `done` after explicit approval. |
| `garyx task stop <task_ref>` | Interrupt the active run on the task's backing thread, if one exists, then release the task back to a non-running state. |
| `garyx task delete <task_ref>` | Delete task metadata so it leaves task lists. The backing thread and transcript are retained for audit. |
| `garyx task assign <task_ref> <principal>` | Hand a task to an agent or human (`agent:<id>`, `human:<id>`, or a bare agent id). Assigning an agent starts a run on the task. |
| `garyx task set-title / reopen / history` | Rename, reopen, or inspect task history. |

## Agents and teams

| Command | Use it for |
| --- | --- |
| `garyx agent list / get / create / update / upsert / delete` | CRUD on custom agents. `create/update/upsert` accept `--model`, `--model-reasoning-effort`, `--model-service-tier`, and `--default-workspace-dir <path>` for new bot/task threads. |
| `garyx team list / get / create / update / delete` | CRUD on agent teams. |

## Workflows

| Command | Use it for |
| --- | --- |
| `garyx workflow definition list / get / upsert --file <package>` | Manage global file-backed workflow packages rooted by `garyx.workflow.json`. |
| `garyx task create --workflow <workflow_id> --input '<text>'` | Execute a workflow definition through a Task executor. `--input` is a single plain-text string; a workflow that needs structured data parses it in its first step. |
| `garyx workflow list / get / events / cancel` | Inspect or cancel existing workflow runs. Runs are execution records under Tasks, not standalone product entries. |

## Tools

| Command | Use it for |
| --- | --- |
| `garyx tool search <query...>` | Search the web through the Gemini provider-native search path. |
| `garyx tool image <prompt> --output <path>` | Generate one image through the configured Codex provider. |

## Automations

| Command | Use it for |
| --- | --- |
| `garyx automation list` | List scheduled automations. |
| `garyx automation get <automation_id>` | Show one automation. |
| `garyx automation create --label <label> [--prompt <text>] [--workspace-dir <path> \| --thread-id <thread_id>] [schedule flags]` | Create a scheduled prompt. Reads stdin when `--prompt` is omitted. `--workspace-dir` creates a fresh automation thread per run; `--thread-id` sends each run into an existing thread. |
| `garyx automation update <automation_id> [--label <label>] [--prompt <text>] [--workspace-dir <path> \| --thread-id <thread_id>] [schedule flags]` | Update prompt, agent, workspace, target thread, schedule, or enabled state. |
| `garyx automation run <automation_id>` | Run an automation immediately. |
| `garyx automation pause / resume <automation_id>` | Disable or enable an automation. |
| `garyx automation activity <automation_id>` | Show recent automation runs. |
| `garyx automation trigger data list / create / enable / disable / delete` | Manage data-change triggers that create Garyx tasks. |
| `garyx automation delete <automation_id>` | Delete an automation. |

Schedule flags include `--every-hours <N>`, `--daily-time HH:MM`,
`--weekday mon`, `--timezone <tz>`, `--once-at <time>`, and
`--schedule-json <json>`. `--schedule-json` also accepts monthly schedules such
as `{"kind":"monthly","day":24,"time":"08:00","timezone":"Asia/Shanghai"}`.

## App Database

| Command | Use it for |
| --- | --- |
| `garyx db table list / schema <table>` | List dynamic SQLite tables or inspect one schema. |
| `garyx db table create <table> --field name:TEXT` | Create a STRICT SQLite table. Names are real SQL identifiers and must be snake_case. |
| `garyx db table drop <table>` | Drop a dynamic table. |
| `garyx db field add <table> <field> <TYPE>` | Add a column. Types are `TEXT`, `INTEGER`, `REAL`, `BLOB`, and `ANY`. |
| `garyx db field drop <table> <field>` | Drop a column. |
| `garyx db record insert <table> --data '<json>'` | Insert one record through the write API. |
| `garyx db record get / update / delete` | Read, mutate, or delete one record by `id`. |
| `garyx db sql "select ..."` | Run read-only SQL. Write SQL is rejected by the gateway. |
| `garyx db events` | Inspect schema and record mutation events. |

The database is global for the Garyx installation and is stored at
`~/.garyx/data/app-database.sqlite3` by default.

## Diagnostics

| Command | Use it for |
| --- | --- |
| `garyx status` | Show running gateway + channel summary. |
| `garyx doctor` | Run health checks (CLIs found, ports open, config valid). |
| `garyx logs path` | Print the gateway stderr log path. |
| `garyx logs tail [--lines N]` | Tail the gateway stderr log. |
| `garyx logs clear` | Truncate the log file. |
| `garyx bot status <bot_selector>` | Current bot main endpoint and bound thread status. |
| `garyx bot endpoint list [--bot <bot_selector>]` | List discovered channel endpoints, including groups, channels, DMs, and their bound threads. |
| `garyx bot endpoint bind --endpoint <endpoint_key> --thread <thread_id>` | Bind or rebind an exact channel endpoint to an existing thread through the running gateway. |
| `garyx bot endpoint detach --endpoint <endpoint_key>` | Detach an exact channel endpoint from its current thread. |

## Updates

| Command | Use it for |
| --- | --- |
| `garyx update` | Download the latest release binary from GitHub and replace the running one. |

## Misc

| Command | Use it for |
| --- | --- |
| `garyx message [--bot <selector>] [--image <path> \| --file <path>] [text]` | Send text, one local image, or one local file via a bot without starting an agent run. Text is used as the attachment caption. If `--bot` is omitted, Garyx reads `GARYX_BOT` or `GARYX_CHANNEL` + `GARYX_ACCOUNT_ID`; otherwise it errors. File attachments are currently supported for Telegram bots. |

## Where to go next

- [Configuration](/configuration) — every dotted path you can pass to `config get/set`
- [Service manager](/reference/service-manager) — under-the-hood for `gateway install`
