# Repository Contracts

These contracts describe source-of-truth boundaries that should not be
reinterpreted in feature code.

## Source Of Truth

- Config: `~/.garyx/garyx.json`.
- Channel accounts: `channels.<channel_id>.accounts[...]`
  (`channels.api.accounts[...]` for API).
- Configured bot account `config` is ordinary application state. Mobile and
  desktop should not add token-specific merge, redaction, or preservation paths
  beyond keeping real secrets out of committed fixtures.
- Thread records and known endpoint state: `garyx-router`.
- MCP schema and tool behavior: `garyx-gateway/src/mcp.rs`.
- Provider session behavior: `garyx-bridge`.
- Channel/plugin stream presentation policy, buffering, and tool-call display
  helpers: `garyx-channels/src/plugin_tools.rs`.

## Recent Threads And Active Runs

- Mobile recent-thread lists read the gateway SQLite `recent_threads`
  projection only.
- Keep `recent_threads` current by writing it from the thread-store write path;
  do not make `GET /api/recent-threads` rescan router/thread files.
- Provider bridge run persistence must use the same projecting thread store as
  the gateway/router so active run state updates are dual-written into
  `recent_threads` at write time.
- Do not repair stale `active_run_id` or `run_state` in read routes.
- Startup reconciliation may repair historically stale active-run projection
  rows as a data migration, but steady-state correctness must come from the
  thread-store write path.

## Provider And Channel Behavior

- Telegram uses the throttled plugin stream policy: assistant text can stream
  through 300ms edit coalescing while top-level tool calls flush immediately.
- Discord uses the buffered plugin stream policy: assistant text deltas wait
  until a top-level tool call starts or the run finishes; rapid tool-call
  placeholder updates are coalesced with a one-second minimum interval.
- When a queued user message is acknowledged mid-stream, Discord finalizes the
  current reply segment and starts subsequent assistant output in a new message.
- Discord REST writes retry 429, transient network, and 5xx responses with
  backoff.
- Garyx in-process native model providers load Garyx-managed Skills from
  `~/.garyx/skills` and managed MCP from gateway-injected
  `remote_mcp_servers`; they should not read downstream Claude/Codex Skill or
  MCP config files.
