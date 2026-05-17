# Garyx Configuration

Garyx reads its main configuration from:

```text
~/.garyx/garyx.json
```

The file is JSON. Values can reference environment variables with
`${NAME}` or `${NAME:-default}`.

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
| `workspace_dir` | Default execution directory path for new threads from this account. Takes priority over the selected Agent's `default_workspace_dir`. |
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

Outbound Telegram text is sent with Bot API `parse_mode=MarkdownV2`. Garyx
translates common assistant Markdown, including bold, italic, inline code,
fenced code blocks, links, and reserved-character escaping. If Telegram rejects
the MarkdownV2 entity parsing for a send or edit, Garyx logs the failure and
retries that same message as plain text without `parse_mode`.

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
            "uin": "${WEIXIN_UIN}",
            "streaming_update": true
          }
        }
      }
    }
  }
}
```

`streaming_update` defaults to `true`. It enables Weixin in-place updates for
streamed assistant text by reusing one `client_id` with
`message_state=1 -> 2`. Set it to `false` to fall back to the legacy path where
each flushed chunk is sent as an independent finished message.

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
garyx gateway restart --no-wake
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

### Updating plugins

`garyx plugins update [<name>]` refreshes a subprocess channel
plugin in place. With no `<name>` it iterates every installed
subprocess plugin (continue-on-error).

```bash
# Update a single plugin to the latest version its manifest_url advertises.
garyx plugins update example-plugin

# Pin to an explicit version.
garyx plugins update example-plugin --version 0.1.16

# Reinstall the current version (handy after a packaging fix).
garyx plugins update example-plugin --force

# Dry-run: print "current vs latest" without downloading.
garyx plugins update example-plugin --check

# Update from a specific bundle on disk (local build) or URL.
garyx plugins update example-plugin --from ./target/release/garyx-plugin-example-plugin
garyx plugins update example-plugin --from https://example.test/foo-bundle.tar.gz

# Update every installed plugin.
garyx plugins update
```

Built-in channels (`telegram`, `feishu`, `weixin`) are compiled into
the garyx binary; `garyx plugins update <builtin>` errors with a
redirect to `garyx update`.

Restart the gateway after each manual update so the new binary is
picked up:

```bash
garyx gateway restart --no-wake
```

Plugins that opt in to the silent auto-updater (see below) get this
restart for free — the gateway hot-swaps the subprocess on the next
auto-update tick.

### Silent auto-update

When the gateway is running it also runs a background auto-updater
that mirrors `garyx-desktop`'s built-in app updater: an initial check
~8 seconds after boot, then a recurring check every 6 hours by
default. For each installed plugin with a declared `[update]` block,
it discovers the latest version and — when one is available AND the
plugin author opted in via `[capabilities].survives_respawn = true`
— downloads, sha256-verifies, atomically promotes, and hot-replaces
the running subprocess via the §9.4 respawn path, all without
restarting the gateway.

Plugins that have not opted into `survives_respawn` still benefit
from the discovery loop: the host warn-logs a one-line notice when
a new version becomes available. The auto-updater does **not**
download or promote the new bundle in that case — both the on-disk
install and the running subprocess stay on the old version until
the operator manually runs `garyx plugins update` + `garyx gateway
restart`. The opt-in is conservative because some plugins keep
per-account dedup state in memory; respawning them would re-deliver
historical messages unless they persist that state across restarts.
Plugin authors set the flag only after they've verified their
plugin resumes cleanly from a child-process restart.

Configuration knobs in `~/.garyx/garyx.json`:

```json
{
  "plugins": {
    "auto_update": true,
    "auto_update_check_interval_secs": 21600
  }
}
```

- `auto_update` (bool, default `true`) — master switch. `false`
  disables the background loop entirely; manual `garyx plugins
  update` still works.
- `auto_update_check_interval_secs` (u64, default `21600` = 6 h) —
  seconds between checks. Clamped at a 60 s floor to keep manifest
  hosts from being hammered.

Plugin authors opting in:

```toml
[capabilities]
delivery_model = "pull_explicit_ack"
# I (the plugin author) certify that respawning my subprocess does
# not duplicate inbound messages to the gateway. Typically this
# requires persisting per-account cursors / dedup state on disk so
# the new child resumes from the same logical position as the old.
survives_respawn = true
```

### Declaring an update source in `plugin.toml`

Plugin authors who want their bundle to be updatable via `garyx
plugins update` add an `[update]` section. The host carries the
section verbatim into `~/.garyx/plugins/<id>/plugin.toml` during
install, so a single declaration is enough.

```toml
[update]
# Required when --version is omitted. See "manifest_url JSON schema"
# below for the response shape.
manifest_url = "https://example.com/garyx/plugins/{id}/latest.json"

# Required. Templated archive URL.
url_template = "https://example.com/garyx/plugins/{id}/{version}/garyx-plugin-{id}-{version}-{target}.tar.gz"

# Optional. Defaults to "{url}.sha256". Empty string disables.
checksum_url_template = "{url}.sha256"

# Optional. Defaults to "{id}/garyx-plugin-{id}".
binary_in_archive = "{id}/garyx-plugin-{id}"
```

Placeholders: `{id}`, `{version}`, `{target}` (one of
`linux-x86_64`, `linux-aarch64`, `mac-x86_64`, `mac-aarch64`), and
`{url}` (only valid in `checksum_url_template`, expands to the
rendered `url_template`). Unknown placeholders fail at manifest
load time, not at HTTP-404 time.

### `manifest_url` JSON schema

The host fetches `manifest_url` (HTTP GET, 30s timeout) and parses
the response as JSON. The minimum contract:

```json
{
  "version": "0.1.16"
}
```

- `version` (required, string) — the latest published version. Used
  to resolve `--version` defaults and `--check` comparisons.
- Any other top-level field is currently ignored.

Compatibility promise from the host:

1. Future fields are **additive only** — existing fields keep their
   semantics and types.
2. Older hosts silently ignore newer fields, so plugin authors can
   start emitting additional metadata whenever they want without
   waiting for a coordinated rollout.

A `{"version": "..."}` document will keep working indefinitely.

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

## Automations

Scheduled automations are managed from the CLI:

```bash
garyx automation list
garyx automation create --label "Daily triage" --prompt "Summarize repo state" --workspace-dir /path/to/repo --every-hours 24
garyx automation update <automation-id> --daily-time 09:30 --weekday mon --weekday fri --timezone Asia/Shanghai
garyx automation pause <automation-id>
garyx automation resume <automation-id>
garyx automation run <automation-id>
garyx automation delete <automation-id>
```

The MCP surface intentionally does not expose automation management tools.

## App Database

Garyx includes a global SQLite-backed app database for agent-managed tables and
records. It is stored under the configured sessions data directory as
`app-database.sqlite3` (`~/.garyx/data/app-database.sqlite3` by default).

Agents use the CLI:

```bash
garyx db table create contacts --field name:TEXT --field score:REAL
garyx db record insert contacts --data '{"name":"Test User","score":9.5}'
garyx db sql "select id, name, score from contacts"
```

Read queries use SQL and the gateway rejects write SQL. Schema and record
writes go through the `garyx db table`, `garyx db field`, and `garyx db record`
commands. Data-change triggers live under Automation and can be managed with
`garyx automation trigger data ...`; a trigger creates a Garyx task when the
configured table event fires. Scheduled automations and data-change
automations are two trigger mechanisms under the same Automation domain.

## Agents and Teams

Each channel account can set `agent_id`.

Use a built-in provider agent:

```json
{ "agent_id": "claude" }
```

Custom agents can set `provider_type` to `claude_code`, `claude_tty`,
`codex_app_server`, `gemini_cli`, or `gpt`. `claude_tty` uses the local Claude
CLI's interactive terminal mode inside the gateway and keeps the same
thread/session model as the regular Claude provider.

Custom agents may also set `model`, `model_reasoning_effort`, and
`model_service_tier`. These values are injected into the thread runtime metadata
when the agent is selected, so provider-specific defaults can be overridden per
agent.

`gpt` is the OpenAI GPT model backend running on Garyx's in-process agent loop.
It is not exposed as a built-in agent. Create a custom agent with
`provider_type: "gpt"` to select it:

```json
{
  "agent_id": "gpt-reviewer",
  "display_name": "GPT Reviewer",
  "provider_type": "gpt",
  "model": "gpt-5.5"
}
```

The GPT provider uses Codex-compatible auth by default. It checks
`CODEX_API_KEY`, then `OPENAI_API_KEY`, then Codex auth at
`$CODEX_HOME/auth.json` or `~/.codex/auth.json`. Codex auth files with
`OPENAI_API_KEY` use the OpenAI Responses API; auth files with
`tokens.access_token` use the ChatGPT Codex backend and forward the stored
ChatGPT account id when present. Codex `agent_identity`-only auth is not
duplicated by the GPT backend.

Optional GPT-provider fields on an agent/provider config:

```json
{
  "provider_type": "gpt",
  "default_model": "gpt-5.5",
  "model": "",
  "model_reasoning_effort": "medium",
  "model_service_tier": "",
  "auth_source": "codex",
  "base_url": "",
  "codex_home": "",
  "max_tool_iterations": 32,
  "request_timeout_seconds": 300
}
```

`model` can be left empty to use the provider default. The gateway exposes
GPT model choices through `/api/provider-models/gpt` by reading the same Codex
`/models` catalog used by the local Codex CLI. If that request is unavailable,
Garyx falls back to a minimal copy of Codex's bundled model catalog so the
picker can still show the Codex default (`gpt-5.5`) and the standard GPT coding
models. `garyx_native`, `garyx`, and `native` are accepted as legacy provider
slug aliases for `gpt`; they do not create agent id aliases.
`model_reasoning_effort` accepts the reasoning
levels advertised by the selected Codex model, for example `low`, `medium`,
`high`, or `xhigh`; lower values favor faster responses, while higher values
spend more reasoning time. `model_service_tier` accepts the selected model's
advertised service tier ids, for example `priority` for Codex's Fast tier; leave
it empty to use the backend default.

The `/goal <objective>` native command sets a durable thread goal and enables
loop mode. `/goal` shows the current goal; `/goal pause` pauses it; `/goal
resume` resumes it; `/goal clear` clears it and disables loop mode. While a
goal is active, Garyx keeps loop mode running until the provider marks the goal
completed with its `update_goal` tool, the goal is paused/cleared, the run is
interrupted, or the loop hits its safety limit.

Use a custom agent or an agent team by setting the same `agent_id` used in your
Garyx agent/team configuration. The CLI account setup flow can also prompt for
an agent when `--agent-id` is omitted.

Custom agents can also store an optional `default_workspace_dir`. It is a path
string, not a Workspace entity. New bot/channel threads use
`account.workspace_dir` first, then the Agent default, then the provider's
home/root fallback. Direct task creation uses explicit
`garyx task create --workspace-dir` first, then the assignee Agent default,
then the same fallback.

New local threads and task backing threads can opt into a managed Git worktree:

```bash
garyx thread create --workspace-dir /path/to/repo --worktree
garyx task create --title "Investigate" --workspace-dir /path/to/repo --worktree --notify none
```

`--worktree` requires `--workspace-dir` to be the Git repository root and the
repository must have at least one commit. Garyx creates the isolated checkout
under `~/.garyx/worktrees/<repo-hash>/<thread-id-safe>` from the selected
repository's current `HEAD`, records the source repo, branch, base commit,
generated branch, and worktree path on the thread, and then runs the provider
with the worktree path as `workspace_dir`. Garyx does not auto-delete these
worktrees when a thread or task is deleted.

Custom agents may include `avatar_data_url`, a small image data URL used by
desktop surfaces for the agent avatar. Omit it or set it to an empty string to
use the generated initials fallback.

Custom agent model selection is provider-specific. Claude and Codex use their
provider defaults in the desktop app. Gemini only shows a model picker when the
gateway can discover models from the local Gemini ACP process; Garyx does not
use a Gemini API key to populate that list.

## Tasks

Tasks are stored as metadata on their backing Garyx thread. Stopping a task
interrupts any active run on that thread and releases the task back to a
non-running state. Deleting a task removes that metadata so the task disappears
from task lists; the backing thread and transcript are retained for audit.

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

Gateway runtime settings are edited from the Gateway tab in the desktop
Settings view.
Thread history is persisted through transcript records; backend selection is no
longer exposed as a gateway setting.

The desktop app mirrors its current view into the window URL hash. For example,
thread pages use `#/thread/<thread-id>`, new-thread drafts can use
`#/new?workspace=<path>`, and settings pages use `#/settings/<tab>`. This is a
desktop navigation aid only: it lets Command-R reload the app without losing
the active thread, draft directory, or settings tab. The registered `garyx://`
protocol uses matching paths such as `garyx://thread/<thread-id>` and
`garyx://new?workspace=<path>`.

The desktop sidebar shows only folders that were manually added in the Mac app.
Thread history can still carry `workspace_dir` paths from the gateway, but those
paths do not appear as sidebar projects until the user adds the folder. The
project row can be collapsed or expanded from the sidebar, and the row menu can
remove the folder from the desktop list without deleting transcripts.

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

Gateway restart commands restart the installed binary only; they do not build
from a source checkout. When testing a local code change, install or copy the
new `garyx` binary into the service's PATH first, then restart.

```bash
garyx config show
garyx config validate
garyx doctor
garyx channels list
garyx channels add telegram main --token "$TELEGRAM_BOT_TOKEN"
garyx channels login feishu --account main
garyx plugins list
garyx gateway restart --no-wake
garyx logs tail
```

By default `garyx logs path` and `garyx logs tail` read the managed gateway's
stderr log at `~/.garyx/logs/stderr.log`, which is where runtime warnings and
provider/channel errors are written.

`garyx config validate` and `garyx doctor` both check channel account payloads
beyond basic JSON parsing. They flag stale accounts with `config: null`, invalid
built-in channel credentials, and missing required fields declared by installed
channel plugin manifests. The gateway settings API applies the structural part
of this guard before persisting updates, so desktop or HTTP clients cannot
overwrite an existing account with a missing, `null`, or schema-incomplete
`config` payload.

## Testing a Source Checkout

```bash
cargo test --workspace --all-targets
cd desktop/garyx-desktop && npm run build:ui
cd desktop/garyx-desktop && npm run test:smoke
```
