# Gateway Startup Critical Path

Date: 2026-07-07

## Problem

`garyx gateway restart` can report:

```text
gateway did not start listening on port 31337 in time
```

even when launchd has already started the gateway process and the gateway binds
the port shortly afterward.

The root issue is that the current startup path performs provider, channel, and
subprocess plugin initialization before the HTTP listener is bound. Slow external
network checks or plugin preflight therefore make the whole gateway look dead to
the CLI, desktop, mobile, and any local tool that only needs the control plane.

## Current Startup Shape

The CLI command roughly does this:

```text
load config
RuntimeAssembler::assemble()
  open local stores
  initialize MultiProviderBridge from config
  load and start cron
  build AppState
  reload bridge after agent profile sync
start config hot reload
register built-in channel plugins
register subprocess channel plugins
start gateway auto-update
Gateway::serve()
  start gateway runtime
  bind 31337
  warm cache
  serve HTTP
```

Important code locations:

- `garyx/src/commands/gateway.rs`: calls `RuntimeAssembler::assemble()` before
  serving HTTP.
- `garyx/src/runtime_assembler.rs`: initializes local stores, bridge, cron, and
  app state.
- `garyx-gateway/src/server.rs`: binds the TCP listener inside `Gateway::serve`.
- `garyx-channels/src/feishu.rs`: channel startup fetches access tokens and
  spawns WebSocket listeners.
- `garyx/src/channel_plugin_host.rs`: subprocess channel plugin registration
  runs plugin preflight before live registration.

## What Must Block The HTTP Listener

Only the minimum needed to answer basic local control-plane requests should
block `listen(31337)`:

- parse config and runtime overrides.
- open the local state required to build `AppState`.
- construct the HTTP router and route cells.
- bind the listener.

After this point `/health` should be able to return, and `/api/status` should be
able to report that providers, channels, plugins, cron, and restart wake are
still starting.

## What Can Be Lazy Or Background

### Providers

Provider definitions need to be visible early, but expensive provider readiness
checks do not need to complete before HTTP is available.

Recommended behavior:

- register configured providers as `configured` or `lazy`.
- verify binaries, app-server clients, auth state, and model availability in a
  background warmup task.
- also verify on first run if warmup has not completed.
- expose readiness and last error in `/api/status`.

This applies to providers such as Claude Code, Codex app server, Traex,
Antigravity, and remote API providers.

### Channels

External channel startup should not block local HTTP readiness.

Recommended behavior:

- register channel configurations immediately as `starting`.
- spawn startup for Telegram, Discord, Feishu, Weixin, and similar channels
  after HTTP bind.
- let token refresh, WebSocket connection, command menu sync, and polling loops
  report readiness asynchronously.
- keep channel-specific failures isolated so one slow channel does not delay
  local CLI or app control-plane calls.

### Subprocess Plugins

Subprocess plugin discovery and preflight should not block the listener.

Recommended behavior:

- discover manifests in the background.
- mark discovered plugins as `preflighting`, `running`, or `failed`.
- put a bounded timeout around each preflight.
- avoid starting live lifecycle children until preflight succeeds.
- keep plugin stderr and compliance checks out of the listener critical path.

The Minolab plugin is the observed example, but the rule should apply to every
subprocess channel plugin.

### Cron And Restart Wake

Cron and restart wake need persistent state, but they do not need to run before
HTTP exists.

Recommended behavior:

- load cron metadata early enough for status display.
- start cron execution after HTTP bind.
- enqueue restart wake targets after HTTP bind.
- throttle wake-all restoration with an explicit concurrency cap.
- prefer explicit or scoped restart wake for manual restarts; default wake-all
  should not be allowed to saturate startup.

## Desired Startup Model

```text
Phase 0: config and local state
  read config
  open stores
  build AppState with status cells

Phase 1: control-plane ready
  build router
  bind 31337
  /health returns lightweight OK
  /api/status reports startup phases

Phase 2: background warmup
  provider warmup
  channel startup
  subprocess plugin discovery/preflight/start
  cron start
  restart wake dispatch

Phase 3: steady state
  providers/channels/plugins report ready or failed
  failed integrations can retry without affecting /health
```

## Health And Status Contract

`/health` should stay cheap and reliable:

- no external network.
- no subprocess calls.
- no provider readiness checks.
- no long DB scans.
- no locks shared with provider run execution when avoidable.

`/api/status` can be richer:

```json
{
  "gateway": "ready",
  "startup": {
    "phase": "background_warmup",
    "http_bound_at": "timestamp",
    "started_at": "timestamp"
  },
  "providers": {
    "codex_app_server": { "state": "warming" },
    "traex": { "state": "ready" }
  },
  "channels": {
    "feishu": { "state": "connecting" },
    "telegram": { "state": "running" }
  },
  "plugins": {
    "minolab": { "state": "preflighting" }
  },
  "restart_wake": {
    "queued": 1,
    "running": 0
  }
}
```

Exact field names can follow existing status API conventions; the important
contract is that control-plane readiness is separated from integration
readiness.

## CLI Behavior

`garyx gateway restart` should distinguish these cases:

- launchd failed to start the process.
- process started but port has not been bound yet.
- port is bound but `/health` is timing out.
- `/health` is OK but background integrations are still warming.

The command should not report a generic startup failure when the process is
alive and the only missing piece is non-critical warmup. It should either wait
with a clearer phase message or return success once the control plane is ready.

## Implementation Notes

The smallest practical refactor is not a second process. It is a staged startup
within the existing process:

1. Build `AppState` with status cells for providers, channels, plugins, cron,
   and restart wake.
2. Bind HTTP immediately after the minimal state is ready.
3. Spawn a supervised background startup task from `run_gateway`.
4. Move channel/plugin startup from the pre-listen path into that task.
5. Add readiness state updates and bounded timeouts around each integration.
6. Keep restart wake dispatch behind an explicit semaphore.

The larger future direction is to split the heavy agent/runtime data plane from
the HTTP control plane, but staged startup gets most of the restart reliability
benefit without a process boundary.
