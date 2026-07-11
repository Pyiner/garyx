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
| `agent_id` | Agent used for new inbound threads. |
| `workspace_dir` | Default execution directory path for new threads from this account. Takes priority over the selected Agent's `default_workspace_dir`. |
| `workspace_mode` | Optional workspace mode for new inbound threads from this account: `local` or `worktree`. Defaults to `local`. |
| `config` | Channel-specific fields declared by the built-in channel or plugin. |

The desktop Add Bot flow validates account connectivity through the gateway
before writing the account. Telegram verifies the bot token with `getMe`;
Discord verifies the bot token with `users/@me`; Feishu/Lark verifies app
credentials by requesting a tenant access token. Channels without a safe probe
explicitly report the validation as skipped.

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
During streaming, top-level tool calls are shown as short numbered progress
placeholders and flush immediately, while assistant text edits are coalesced at
roughly 300ms intervals. Garyx hides child-agent and internal
planning/reasoning tool events from the Telegram chat.

### Discord

```json
{
  "channels": {
    "discord": {
      "accounts": {
        "main": {
          "enabled": true,
          "agent_id": "claude",
          "config": {
            "token": "${DISCORD_BOT_TOKEN}",
            "require_mention": true
          }
        }
      }
    }
  }
}
```

Discord connects through the Gateway API and sends replies through the REST
message APIs. Direct messages are accepted without a mention. Server channels
require a bot mention by default; set `require_mention` to `false` to allow
free-response server channels. Assistant text deltas are buffered and merged
until a top-level tool call starts or the run finishes. Tool calls use the same
numbered progress placeholders as Telegram; rapid tool placeholder updates are
coalesced to the latest state with a one-second minimum interval. If a queued
user message is acknowledged while a response is still streaming, Discord
finalizes the current reply segment and starts later assistant output in a new
message; runtime-only tool placeholders are deleted during that split.
Discord REST writes retry 429 responses using Discord's `Retry-After` /
`retry_after` delay, and retry transient network or 5xx failures with backoff
before surfacing a delivery failure.
Child-agent and internal planning/reasoning tool events stay hidden. Local and
remote Markdown image references are sent as Discord attachments, generated
image results are sent as files, and inbound Discord image/file attachments are
downloaded to local temp files before the agent run. Outbound messages use safe
`allowed_mentions` defaults: user pings and reply pings are allowed, while
`@everyone`, `@here`, and role pings are blocked.

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

Plugins drive their own upgrades. Each plugin runs a self-update
tick that polls *its own* update server on its own schedule; when
the server advertises a higher version, the plugin sends the host a
`request_self_replace` reverse RPC carrying `{archive_url, sha256,
version, request_id}`. The host performs the safe-swap pipeline —
strict-greater version gate, archive sha256 verification, archive's
embedded `plugin.toml` id/version validation, stream-idle gate, swap
barrier, atomic rename, respawn — and returns a structured decision.

The host no longer runs a per-plugin update poll loop. The
`garyx plugins update` CLI command was retired with the loop; the
only manual escape hatch is sideloading via `garyx plugins install
--force <PATH>` against a local binary. Plugin authors who want a
"force update now" knob expose it inside the plugin itself.

#### Host-side master switch

`~/.garyx/garyx.json::plugins.auto_update` is the kill switch. When
`false`, the host refuses every incoming `request_self_replace` RPC
with `{decision: "refused", reason: "master_disabled"}` and the
plugin's tick logs a single info line and retries on its next
interval. The flag is read once at handler-construction time; flip
it via `garyx auto-update disable --plugin` / `garyx auto-update
enable --plugin`, then restart the gateway for the change to
propagate to running plugin handlers (a future iteration can swap
the read to an `Arc<AtomicBool>` for hot-reload).

```bash
garyx auto-update status
garyx auto-update disable --plugin    # host refuses request_self_replace
garyx auto-update enable --plugin     # host accepts again
```

Built-in channels (`telegram`, `discord`, `feishu`, `weixin`) are compiled into
the garyx binary; `garyx plugins update <builtin>` errors with a
redirect to `garyx update`.

#### Decision taxonomy

Every `request_self_replace` returns one of:

| `decision`     | `reason`                                                                            | Plugin should                                  |
|----------------|-------------------------------------------------------------------------------------|------------------------------------------------|
| `applied`      | —                                                                                   | nothing — the plugin process is being killed   |
| `refused`      | `downgrade` / `already_current`                                                     | cache the advertised version as "no upgrade"   |
| `refused`      | `master_disabled`                                                                   | retry next tick; flag may flip back            |
| `refused`      | `no_survives_respawn`                                                               | give up; needs operator action                 |
| `refused`      | `id_mismatch` / `version_mismatch` / `invalid_params` / `plugin_not_registered`     | bug — log and stop retrying                    |
| `deferred`     | `stream_active`                                                                     | retry next tick                                |
| `swap_failed`  | `sha256` / `download` / `extract` / `manifest` / `promote` / `respawn`              | retry next tick                                |
| `in_progress`  | —                                                                                   | retry next tick (concurrent swap in flight)   |

In the `applied` path the host respawns the plugin before the RPC
response is written, so the caller never observes "applied" —
useful only for host-side tracing.

#### Plugin author contract

The plugin opts in to host-driven respawn by declaring it can
survive being killed mid-flight and resumed from disk:

```toml
[capabilities]
delivery_model = "pull_explicit_ack"
# Author certifies the subprocess can be killed at any time and
# resumed cleanly from disk — typically by persisting per-account
# cursors / dedup state across restarts. Set this only after you
# have verified your plugin handles respawn without re-delivering
# historical messages.
survives_respawn = true
```

The host refuses any `request_self_replace` when this flag is
false (`reason: no_survives_respawn`).

The plugin's tick is plugin-internal — implement whatever cadence,
release-source, and version-pinning rules suit your release
discipline. A typical pattern: tick every 6 h by default with an
env-var overridable interval, use strict-greater version compare,
and fetch a target-aware archive URL from a plugin-server endpoint.

#### Plugin update server contract

The host does not care where archives come from — the plugin
fetches the `(version, archive_url, sha256)` triple itself and
passes it through. Any HTTPS endpoint that returns

```json
{
  "version": "0.1.35",
  "archive_url": "https://your-cdn.example.test/.../garyx-plugin-foo-0.1.35-aarch64-apple-darwin.tar.gz",
  "sha256": "56e8…aa9e8"
}
```

works. The archive must be a `.tar.gz` containing a `plugin.toml`
whose `[plugin]` id matches the calling plugin's registered id and
whose version matches the manifest's `version` (the host checks
both before promoting; mismatches return `refused/id_mismatch` or
`refused/version_mismatch`).

A minimal reference server can serve manifests sourced from
per-target env vars (`PLUGIN_RELEASE_<PLUGIN_ID>_<TARGET>`)
with no DB or auth — see your plugin author's release
infrastructure for the actual endpoint.

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
garyx automation create --label "Thread check-in" --prompt "Post the scheduled update" --thread-id thread::example --every-hours 24
garyx automation update <automation-id> --daily-time 09:30 --weekday mon --weekday fri --timezone Asia/Shanghai
garyx automation update <automation-id> --schedule-json '{"kind":"monthly","day":24,"time":"08:00","timezone":"Asia/Shanghai"}'
garyx automation update <automation-id> --thread-id thread::example
garyx automation pause <automation-id>
garyx automation resume <automation-id>
garyx automation run <automation-id>
garyx automation delete <automation-id>
```

By default a scheduled automation creates a fresh automation thread for each
run using `--workspace-dir`. Passing `--thread-id` instead binds the automation
to an existing Garyx thread; each scheduled or manual run sends the configured
prompt into that thread exactly like a user message and keeps the transcript
in one conversation. A thread-bound automation always uses the thread's own
workspace — combining `--thread-id` with an explicit `--workspace-dir` is
rejected.

Automation schedules can be represented as hourly intervals, daily or weekday
cron-style runs, one-shot timestamps, or monthly day-of-month runs. The mobile
app presents these as repeat, date/day, and time controls; monthly schedules run
on the selected calendar day in the selected timezone.

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

## Agents

Each channel account can set `agent_id`.

Use a built-in provider agent:

```json
{ "agent_id": "claude" }
```

Custom agents can set `provider_type` to `claude_code`, `codex_app_server`,
`gemini_cli`, `gpt`, `anthropic`, or `google`. `claude_tty` is deprecated and
is treated as `claude_code` when encountered in older records.

Claude has one provider path: the Claude Agent SDK. Configure which executable
the SDK launches with `agents.claude`:

```json
{
  "agents": {
    "claude": {
      "provider_type": "claude_code",
      "claude_cli_mode": "native",
      "claude_cli_path": ""
    }
  }
}
```

`claude_cli_mode` accepts `native` or `cctty`. `native` is the default and lets
the SDK discover and launch the original `claude` executable; `cctty` uses
Garyx's embedded terminal-wrapper runner through the installed `garyx` binary.
`claude_cli_path` is optional and overrides the executable path for either mode,
for example when testing an external wrapper. The same setting is available
from the Mac app's Providers > Claude Code Configure dialog and from the CLI:

```bash
garyx config claude-cli --mode native --clear-path
garyx config claude-cli --mode cctty
garyx config claude-cli --mode cctty --path /opt/garyx/bin/custom-cctty
garyx config provider-model claude_code --claude-cli-mode cctty
garyx config provider-model claude_code --claude-cli-mode cctty --claude-cli-path /opt/garyx/bin/custom-cctty
```

Custom agents may also set `model`, `model_reasoning_effort`, and
`model_service_tier`. These values are injected into the thread runtime metadata
when the agent is selected, so provider-specific defaults can be overridden per
agent.

`gpt`, `anthropic`, and `google` are model backends running on Garyx's
in-process agent loop. They are not exposed as built-in agents or default
runtime providers. Create a custom agent with the model backend provider type
to make one selectable:

```json
{
  "agent_id": "gpt-reviewer",
  "display_name": "GPT Reviewer",
  "provider_type": "gpt",
  "model": "gpt-5.5"
}
```

```json
{
  "agent_id": "anthropic-reviewer",
  "display_name": "Claude Reviewer",
  "provider_type": "anthropic",
  "model": "claude-sonnet-4-6"
}
```

```json
{
  "agent_id": "google-reviewer",
  "display_name": "Gemini Reviewer",
  "provider_type": "google",
  "model": "gemini-3-flash-preview"
}
```

The GPT provider uses Codex-compatible auth by default. It checks
`CODEX_API_KEY`, then `OPENAI_API_KEY`, then Codex auth at
`$CODEX_HOME/auth.json` or `~/.codex/auth.json`. Codex auth files with
`OPENAI_API_KEY` use the OpenAI Responses API; auth files with
`tokens.access_token` use the ChatGPT Codex backend and forward the stored
ChatGPT account id when present. Codex `agent_identity`-only auth is not
duplicated by the GPT backend.

For GPT custom agents, set `"auth_source": "codex"` to reuse the local Codex /
GPT token, or `"auth_source": "api_key"` with `provider_env.OPENAI_API_KEY` to
use a key supplied for that custom provider. `api_key` mode does not fall back
to the Codex token when the key is missing.

The CLI exposes the same path:

```bash
garyx agent create \
  --agent budget-gpt \
  --display-name "Budget GPT" \
  --provider gpt \
  --auth-source codex \
  --system-prompt "Use GPT for this agent."

garyx agent create \
  --agent keyed-gpt \
  --display-name "Keyed GPT" \
  --provider gpt \
  --api-key "${OPENAI_API_KEY}" \
  --system-prompt "Use this provider key."
```

Optional GPT-provider fields on an agent/provider config:

```json
{
  "provider_type": "gpt",
  "default_model": "gpt-5.5",
  "model": "",
  "model_reasoning_effort": "medium",
  "model_service_tier": "",
  "auth_source": "codex",
  "provider_env": {
    "OPENAI_API_KEY": "${OPENAI_API_KEY}"
  },
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

The in-process agent loop exposes Garyx-managed capabilities directly. It reads
enabled skills from `~/.garyx/skills` and makes them available through
`load_skill` / `read_skill_file`. It also exposes Garyx-managed MCP servers
through `list_mcp_tools` / `call_mcp_tool` when those servers are present in the
thread metadata injected by the gateway.

`model_reasoning_effort` accepts the reasoning
levels advertised by the selected Codex model, for example `low`, `medium`,
`high`, or `xhigh`; lower values favor faster responses, while higher values
spend more reasoning time. `model_service_tier` accepts the selected model's
advertised service tier ids, for example `priority` for Codex's Fast tier; leave
it empty to use the backend default.

`anthropic` uses Anthropic Messages API-compatible auth from
`ANTHROPIC_API_KEY` or `CLAUDE_API_KEY`. It can also use
`CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_AUTH_TOKEN`, or `CLAUDE_OAUTH_TOKEN` as a
bearer token. `ANTHROPIC_BASE_URL` or `CLAUDE_BASE_URL` can override the
endpoint, and `ANTHROPIC_VERSION` / `ANTHROPIC_BETA` can override request
headers. `claude_llm` and `claude_model` are accepted as legacy provider slug
aliases.
For a custom `anthropic` agent, the desktop provider manager and CLI
`--api-key` store the key as `provider_env.ANTHROPIC_API_KEY`.

`google` uses Google Gemini API auth from `GEMINI_API_KEY` or
`GOOGLE_API_KEY`. It can also reuse Gemini CLI OAuth by reading
`GEMINI_OAUTH_ACCESS_TOKEN` / `GOOGLE_OAUTH_ACCESS_TOKEN`, or a Gemini CLI OAuth
cache at `$GEMINI_CLI_HOME/oauth_creds.json` or `~/.gemini/oauth_creds.json`.
If the cached token is expired, Garyx can refresh it when
`GEMINI_OAUTH_CLIENT_SECRET` or `GOOGLE_OAUTH_CLIENT_SECRET` is configured;
otherwise refresh the Gemini CLI login first. OAuth requests use the Gemini Code
Assist transport and resolve the Code Assist project with `loadCodeAssist`. Set
`GEMINI_CODE_ASSIST_PROJECT`, `GOOGLE_CLOUD_PROJECT`, or
`GOOGLE_CLOUD_PROJECT_ID` to force a project id. `GEMINI_BASE_URL`,
`GOOGLE_GENERATIVE_AI_BASE_URL`, or `GOOGLE_API_BASE_URL` can override the API
key endpoint; `GEMINI_CODE_ASSIST_BASE_URL`, `GOOGLE_CODE_ASSIST_BASE_URL`,
`CODE_ASSIST_BASE_URL`, or `CODE_ASSIST_ENDPOINT` plus
`CODE_ASSIST_API_VERSION` can override the OAuth endpoint. If a direct
Generative Language bearer token is required, set `GOOGLE_GENERATIVE_AI_ACCESS_TOKEN`.
`gemini_llm`, `google_gemini`, and `gemini_model` are accepted as legacy
provider slug aliases.
For a custom `google` agent, the desktop provider manager and CLI
`--api-key` store the key as `provider_env.GEMINI_API_KEY`.

The gateway exposes built-in picker catalogs for `/api/provider-models/anthropic`
and `/api/provider-models/google`, including per-model reasoning effort choices.
These two providers ignore `model_service_tier`; use `model_reasoning_effort`
for lower-latency or higher-depth model behavior.

Garyx does not expose a persistent `/goal` command or thread-level
auto-continuation loop mode. Use normal thread turns, tasks, or automations for
long-running work.

Use a custom agent by setting the same `agent_id` used in your Garyx agent
configuration. The CLI account setup flow can also prompt for
an agent when `--agent` is omitted.

Custom agents can also store an optional `default_workspace_dir`. It is a path
string, not a Workspace entity. New bot/channel threads use
`account.workspace_dir` first, then the Agent default, then the provider's
home/root fallback. Direct task creation uses explicit
`garyx task create --workspace-dir` first, then the executor Agent default,
then the same fallback.

New local threads and task backing threads can opt into a managed Git worktree:

```bash
garyx thread create --workspace-dir /path/to/repo --worktree
garyx task create --title "Investigate" --agent <agent_id> --workspace-dir /path/to/repo --worktree --notify none
```

Bot accounts can make the same choice for newly created inbound threads:

```json
{
  "channels": {
    "telegram": {
      "accounts": {
        "main": {
          "agent_id": "claude",
          "workspace_dir": "/path/to/repo",
          "workspace_mode": "worktree",
          "config": {
            "token": "${TELEGRAM_BOT_TOKEN}"
          }
        }
      }
    }
  }
}
```

You can also set it from the CLI:

```bash
garyx channels add telegram main --workspace-dir /path/to/repo --workspace-mode worktree --token ${TELEGRAM_BOT_TOKEN}
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

Garyx's in-process model providers do not rely on downstream provider config
files for MCP. They use the same managed MCP entries from `mcp_servers` after
the gateway injects them into the run metadata.

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
Saving the gateway URL/token/header settings first verifies connectivity. Only verified
saves are written to local desktop state and added to the Gateway URL history;
the field can still be edited manually.

Gateway runtime settings are edited from the Gateway tab in the desktop
Settings view.
Thread history is persisted through transcript records; backend selection is no
longer exposed as a gateway setting.

### Gary X Mobile

The iOS app is a direct Garyx gateway client. It does not run model providers
and does not copy provider API keys to the phone. It reuses the desktop app's
gateway connection shape:

- `gatewayUrl`: the URL the phone can reach.
- `gatewayAuthToken`: the token created by `garyx gateway token`.
- `gatewayHeaders`: optional custom HTTP headers for reverse proxies or
  tunnels; the apps edit these as one name/value row per header.

For a physical phone, the gateway must be reachable from the LAN. A managed
macOS gateway service is installed to listen on `0.0.0.0`; if you run the
gateway manually, pass `--host 0.0.0.0` or set `gateway.host` accordingly.
Use the Mac's LAN address in the mobile URL, for example
`http://192.168.1.20:31337`. `http://127.0.0.1:31337` only works from the iOS
simulator running on the same Mac.

The desktop app's Desktop Settings view can generate a Gary X Mobile QR/link:

```text
garyx://mobile/connect?gatewayUrl=...&gatewayAuthToken=...
```

Opening that link on iOS imports the gateway URL, optional custom headers, and
stores the gateway token in the iOS Keychain.

The mobile app mirrors the Mac app's operational surfaces through gateway APIs:
thread chat/history, active agent selection for new threads, task
creation and status changes, automation run-now and pause/enable controls,
Skills visibility, and gateway settings. It intentionally keeps deeper
provider, MCP, channel, and Skill editing on the Mac app where the local
runtime and secrets live.

The desktop Providers tab shows a fixed provider table rather than an arbitrary
add-provider form. `Claude Code`, `Codex`, and `Gemini CLI` are always listed at
the top as built-in provider agents; their Configure dialogs edit desktop-local
auth and environment overrides. The same table also lists Garyx native-loop
model backends (`GPT`, `Claude`, and `Gemini`). Configuring one of those rows
creates or updates its deterministic custom agent (`gpt`, `anthropic`, or
`google`), making it selectable like any other agent.
Clearing the row removes that custom agent. The page does not support adding
extra provider rows; additional named personas still belong in the Agents tab
or CLI custom-agent commands.

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

### Manual update

```bash
garyx update                    # latest from GitHub Releases
garyx update --version 0.1.7    # pin a specific tag
```

### Gateway auto-update

When the gateway runs as a managed service (launchd on macOS, systemd
on Linux), it periodically checks GitHub Releases and self-updates in
the background:

1. fetch the `latest` tag from the configured `github_repo`
2. strict-greater version comparator — never downgrades, never
   replaces a local dev build
3. stream-idle gate — waits until every subprocess plugin reports
   zero active streams for `stream_idle_required_secs` consecutive
   seconds, polled every `stream_idle_poll_interval_secs`, with a
   `stream_idle_max_wait_secs` ceiling
4. download, sha256-verify, codesign (macOS), atomic swap
5. exit cleanly; the OS supervisor restarts on the new binary

Configuration knobs in `~/.garyx/garyx.json`:

```json
{
  "gateway": {
    "auto_update": {
      "enabled": true,
      "check_interval_secs": 21600,
      "stream_idle_required_secs": 60,
      "stream_idle_poll_interval_secs": 5,
      "stream_idle_max_wait_secs": 86400,
      "github_repo": "Pyiner/garyx"
    }
  }
}
```

- `enabled` (bool, default `true`) — master switch.
- `check_interval_secs` (u64, default `21600` = 6 h) — seconds
  between checks. Floor of 60 s to respect GitHub's unauthenticated
  rate limit (60 req/h).
- `stream_idle_required_secs` (u64, default `60`) — how many
  consecutive idle seconds before the swap proceeds. Any new stream
  resets the timer.
- `stream_idle_poll_interval_secs` (u64, default `5`) — how often the
  gate polls.
- `stream_idle_max_wait_secs` (u64, default `86400` = 24 h) — give-up
  ceiling. On timeout the tick is dropped; the next interval retries.
- `github_repo` (string, default `"Pyiner/garyx"`) — override for
  testing against a fork.

**Linux supervisor caveat:** on macOS `launchd` with `KeepAlive=true`
relaunches us automatically after the post-swap exit. On Linux you
need a supervisor with equivalent semantics (systemd user unit with
`Restart=always`, or your container orchestrator's restart policy)
or the gateway won't come back up on the new binary.

### Kill switches

```bash
garyx auto-update status          # show installed / latest + flags
garyx auto-update disable         # disable both gateway + plugin loops
garyx auto-update disable --gateway
garyx auto-update disable --plugin
garyx auto-update enable          # mirror of disable
```

Status reads the on-disk config, not the running gateway's in-memory
state, so a freshly-edited file shows up immediately. Disable /
enable rewrite `garyx.json` atomically and trigger a gateway config
reload — the next tick observes the new flag and either keeps
running or quietly drops out of its loop.

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
