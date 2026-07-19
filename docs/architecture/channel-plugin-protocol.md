# Channel Plugin Protocol (Scheme B)

**Status:** Shipped — this document matches the implementation
(`garyx-channels/src/plugin_host/`, `garyx/src/channel_plugin_host.rs`).
The §-references in code comments (§5.3, §6.3a, §7.1, §9.4, §11.1, …)
point into this document. Wire-name deviations from the original draft
have been folded in: the terminal notification is
`inbound/stream_end` (never `inbound/end`), `deliver_inbound`'s
`local_reply` is always `null` with local replies delivered via
`dispatch_outbound` (§7.1), and the native `dispatch_stream_event`
generation is documented in §7.1a. Sections describing explicitly
deferred work (e.g. built-in dispatcher unification, §12.6
host-proxied ingress) say so inline.
**Owner:** garyx core
**Protocol version:** 1 (`plugin_host::PROTOCOL_VERSION`)

## 1. Goals and non-goals

### Goals

- **Open-source the garyx repo** without shipping proprietary channel code.
  The concrete trigger is `minolab` (a proprietary channel that can't
  ship with garyx itself); the design must support any future
  private/proprietary channel on the same
  footing.
- **Channel code lives in a separate process**, distributed as its own
  binary, under its own license/release cadence, and *loadable at
  runtime* without recompiling or relinking garyx.
- **Desktop UI is schema-driven.** Adding a new channel must not require
  changes to `AddBotDialog` / `EditBotDialog` / the channel logo
  component / the routing config UI. A plugin declares its config fields
  and the UI renders them.
- **Minimal host API surface.** The plugin↔host boundary is the
  narrowest RPC we can get away with; everything else stays in-process.
- **Built-in parity (optional).** `telegram` / `feishu` / `weixin` may
  stay in-process *or* migrate to the protocol. The design does not
  preclude either, and the end-user experience is identical.
- **Crash isolation.** A plugin crash must not take the host down. The
  host supervises, restarts with backoff, and surfaces status to the
  user.
- **Local-first, offline.** No network required for discovery; no
  mandatory plugin registry. Users point garyx at a directory or binary.

### Non-goals

- Not a general-purpose plugin system for *arbitrary* extensions (MCP
  tools, agent providers, UI panels). This protocol is scoped to
  **channels** — i.e. the inbound/outbound messaging surface.
- Not a sandboxing story. Plugins run with the same OS privileges as the
  host. Users install plugins the same way they install any other
  binary; trust is on the user.
- Not a cross-language host FFI. Plugins can be written in any language
  that can speak JSON over stdio; the protocol says nothing about
  Rust/C ABI stability (which in Rust's case is a dead end — see
  §12 "Rejected alternatives").

## 2. Summary

The host (garyx) **spawns each channel plugin as a child process** and
communicates with it over **JSON-RPC 2.0 framed with
`Content-Length:` headers over stdio** (the LSP framing), plus
line-delimited logs on stderr.

The plugin side owns its channel protocol (HTTP, websocket, long-poll,
whatever). The host side owns:

- message routing and thread state (`MessageRouter`),
- provider/agent dispatch (`MultiProviderBridge`),
- persisted config + secrets,
- desktop/CLI UI.

The surface is small by design. The complete RPC list, grouped by
direction:

- **Host → plugin (requests).** `initialize`, `start`, `stop`,
  `shutdown` (lifecycle, §6); `describe` (dry-run discovery, §6.3a);
  `dispatch_outbound` (outbound send, §7.2); `auth_flow/start`,
  `auth_flow/poll`, `auth_flow/cancel` (§8.3).
- **Plugin → host (requests).** `deliver_inbound` (inbound send, §7.1);
  `abandon_inbound` (stream cancellation, §6.2); `record_outbound`
  (§7.3).
- **Host → plugin (notifications).** `inbound/stream_frame`,
  `inbound/stream_end` (server-initiated stream for each `deliver_inbound`,
  §7.1).
- **Plugin → host (notifications, optional).** `register_ingress`
  (advisory URL self-announcement for `push_negative_ack` plugins,
  §11.3).

The three RPCs that carry the bulk of runtime traffic are
`initialize`, `deliver_inbound`, and `dispatch_outbound`. Everything
else is either lifecycle, discovery, or optional.

Configuration is declared by each plugin in a **manifest file**
(`plugin.toml`). The manifest contains a JSON-Schema-like description of
the channel's account fields (tokens, base URLs, enum choices, auth
flows) plus any custom UI hints. Desktop fetches these schemas from the
gateway and renders the add/edit forms generically.

## 3. Terms

| Term | Meaning |
|---|---|
| **Host** | The garyx binary (CLI, gateway, desktop bundle). |
| **Plugin** | A separately-distributed binary implementing one channel. |
| **Manifest** | `plugin.toml` shipped alongside the binary, describing id, version, entry point, config schema, auth flows, capabilities. |
| **Account** | A configured instance of a channel (e.g. "telegram:@myhelperbot"). The host stores accounts; the plugin receives them on `initialize`. |
| **InboundRequest** | A message received *from* the channel, routed to the agent. |
| **Stream event** | Agent-side output streamed back to the plugin as it is produced. |

## 4. Process model

```
                      ┌───────────────────────────────┐
                      │ garyx host (gateway process)  │
                      │  · ChannelPluginManager       │
                      │  · MessageRouter              │
                      │  · MultiProviderBridge        │
                      │  · ChannelDispatcher          │
                      └───────┬─────────────┬─────────┘
                              │stdio        │stdio
                    ┌─────────▼──┐    ┌─────▼──────┐
                    │ plugin A   │    │ plugin B   │      ← separate OS processes
                    │ (minolab)  │    │ (acme-chat)│
                    └────────────┘    └────────────┘
```

- **One process per plugin binary**, not per account. A single
  `minolab` plugin process serves all `minolab` accounts.
- Stdin/stdout carry JSON-RPC. Stderr is **free-form structured logs**
  (one `tracing`-style JSON record per line). Anything on stderr is
  captured by the host and rebroadcast through the normal log sink.
- The host is the **supervisor**: it owns process lifetime. Plugins
  never fork / exec anything themselves in response to protocol
  traffic.
- **Stdio is the ONLY transport.** No Unix sockets, no TCP. This is
  deliberate: stdio can't leak beyond the parent/child pair, survives
  PID namespaces, works identically on macOS/Linux/Windows, and
  requires zero configuration.

### 4.1 Why not gRPC / HTTP / Unix sockets?

- gRPC drags in a protoc toolchain, a code-gen step, and TLS-or-not
  decisions. For this surface area (≤10 methods, one client, one
  server, same machine) it's pure overhead.
- Named pipes / Unix sockets add filesystem state we'd have to clean up
  on crash and aren't portable to Windows without more code.
- Stdio framing is 20 lines of code and is the same protocol every LSP
  tool in the ecosystem already uses.

### 4.2 Why one process per plugin, not per account?

- A plugin typically shares connection pools / caches / rate limiters
  across accounts (see how `MinoLabChannel` today keeps one
  `reqwest::Client` for all accounts).
- Fewer processes → lower startup cost + fewer things to supervise.
- Crash blast radius is "all accounts of one channel", which matches
  user mental model ("my minolab stopped working") better than "one
  account of one channel".

## 5. Wire protocol

### 5.1 Framing

Exactly LSP framing, so any JSON-RPC-over-stdio library works:

```
Content-Length: <byte-count>\r\n
\r\n
<utf-8 json-rpc message>
```

### 5.2 Envelope

Standard JSON-RPC 2.0:

- **Request** (expects response): `{jsonrpc, id, method, params}`
- **Response**: `{jsonrpc, id, result}` or `{jsonrpc, id, error}`
- **Notification** (no response): `{jsonrpc, method, params}` (no `id`)

The host and plugin are **both client and server** on the same pipe.
Requests in either direction use monotonically increasing integer ids
scoped to the sender.

### 5.3 Error codes

Reuses JSON-RPC's reserved range plus our own:

| Code | Name | Meaning |
|---|---|---|
| -32700 | ParseError | Malformed JSON. |
| -32600 | InvalidRequest | Missing jsonrpc field, etc. |
| -32601 | MethodNotFound | Unknown method. |
| -32602 | InvalidParams | Params don't match schema. |
| -32603 | InternalError | Generic internal failure. |
| -32000 | NotInitialized | Any method called before `initialize`. |
| -32001 | AlreadyInitialized | `initialize` called twice. |
| -32002 | AccountNotFound | Unknown account id in params. |
| -32003 | HostShuttingDown | Host is tearing down. |
| -32004 | PluginShuttingDown | Plugin is tearing down. |
| -32005 | ConfigRejected | Lifecycle-time config refusal. Emitted from `initialize` / `describe` when the plugin refuses the entire account config set (protocol version mismatch, schema upgrade required, unsupported manifest capability, etc.). Fatal for that plugin instance until the operator acts. |
| -32006 | Busy | Retryable: receiver is at capacity (e.g. host at `max_inflight_inbound`, §11.2). Caller should back off and retry. |
| -32007 | ChannelConfigRejected | Per-message config refusal from `dispatch_outbound` (e.g. `account_id` disabled since initialize, target chat_id no longer valid). Non-fatal; the host surfaces it as `ChannelError::Config` and the caller's retry policy decides. Distinct from `-32005` so the supervisor does not mistake a bad outbound target for a broken plugin. |
| -32008 | PayloadTooLarge | Fatal, non-retryable: frame exceeds `max_frame_bytes` or inline attachment exceeds its cap (§11.2). Sender MUST split or spill before retrying, never retry verbatim. Kept distinct from `-32006 Busy` because the retry policy is the opposite. |

## 6. Lifecycle (host → plugin)

```
spawn → initialize → start → (running) → stop → shutdown → exit
                                   │
                                   └── crash → host restarts with backoff
```

### 6.1 `initialize`

```jsonc
// request
{
  "method": "initialize",
  "params": {
    "protocol_version": 1,
    "host": {
      "version": "0.1.3",
      "public_url": "https://foo.example.com",
      "data_dir": "/Users/.../garyx",
      "locale": "zh-CN"
    },
    "accounts": [
      {
        "id": "product_ship",
        "enabled": true,
        "config": { /* opaque-to-host, validated against plugin schema */ }
      }
    ]
  }
}
```

- **`accounts[].config` is opaque to the host.** The host treats it as
  `serde_json::Value`. The plugin validates it against its own schema
  and returns `ConfigRejected` on error. This is how a private plugin
  owns its config type end-to-end without leaking it into garyx-models.
- The host never sends secrets to the plugin over stdout unless they
  appear inside `accounts[].config`. Secrets set by the user in desktop
  end up there and only there.

Response:

```jsonc
{
  "result": {
    "plugin": { "id": "minolab", "version": "0.2.0" },
    "capabilities": {
      "outbound": true,
      "inbound": true,
      "streaming": true,
      "images": true,
      "files": true
    }
  }
}
```

### 6.2 `start`, `stop`

No params, no result. Plugin enters/leaves its running state.

`stop` semantics, precise — a single authoritative terminal event per
stream, no splits:

1. Plugin immediately stops accepting *new* upstream work (e.g. halts
   polling, deregisters webhook routes internally, pauses socket
   receive).
2. Plugin does **not** pre-emptively abort in-flight `deliver_inbound`
   streams. It waits for each active `stream_id` to reach `inbound/stream_end`
   from the host, up to the **stop grace deadline**.
3. On grace expiry the plugin sends a new RPC,
   **`abandon_inbound`** (plugin → host), for each still-live
   `stream_id`:

   ```jsonc
   { "method": "abandon_inbound",
     "params": { "stream_id": "str_7a2f…",
                 "reason": "stop_grace_expired" } }
   // result: { "ok": true }
   ```

   Effect: **the host commits to not emitting any further `stream_frame`
   or `inbound/stream_end` for that `stream_id` once it has written the
   `abandon_inbound` response.** The host cancels the router-side task
   if it's still running (best effort — the router's response callback
   becomes a no-op) and marks the stream terminated in its bookkeeping.
   Any host-side state keyed by the stream is released.

   **Post-ACK tombstone rule (normative).** Between the time the plugin
   emits the `abandon_inbound` request and the time it observes the
   host's ACK, the host MAY already have written further
   `stream_frame` / `inbound/stream_end` notifications for that `stream_id`
   into the pipe. Moreover, such frames may already be sitting in the
   SDK's per-`stream_id` serial queue (decoded but not yet dispatched
   to plugin user code), or even executing mid-callback. To keep the
   SDK contract single-authoritative we enforce the tombstone at
   **two** points, not one:

   - **Tombstone set at send time.** The plugin SDK MUST record the
     `stream_id` as *tombstoned* the moment it sends the
     `abandon_inbound` request (not when the ACK arrives).
   - **Codec-layer discard.** The SDK's codec reader MUST silently
     discard any `inbound/stream_frame` or `inbound/stream_end` it reads
     from the pipe for a tombstoned `stream_id`, before dispatch.
   - **Queue purge + execution-time check.** Any frames for the
     tombstoned `stream_id` that were already decoded and enqueued
     onto the per-stream serial queue before the tombstone was set
     MUST be drained and dropped *without invoking user callbacks*.
     The SDK does this by walking the queue on tombstone-set and
     evicting matching entries. As a belt-and-braces safeguard,
     every queue worker re-checks the tombstone set immediately
     before running each queued callback and skips it if the
     `stream_id` has been tombstoned since enqueue.
   - **User callback mid-flight.** If a user `on_delta` / `on_end`
     callback is currently executing when the tombstone is set, the
     SDK lets it run to completion (it cannot be preempted without
     UB), but subsequent queued work is dropped per the previous
     rule. The single synthesized `on_stream_abandoned` is delivered
     only after the current callback returns. This bounds the
     post-ACK observation window to at most one in-flight user
     callback, and that callback's observations predate the ACK.
   - Tombstones are retained for the lifetime of the plugin process
     (see `stream_id` uniqueness rule below); they are not
     GC'd after a window.

   **`stream_id` uniqueness (normative).** A `stream_id` MUST be
   **unique per host process lifetime**, not merely per `account_id`
   or per `deliver_inbound`. The host generates each `stream_id` from
   a process-startup nonce + a monotonic counter (e.g.
   `str_{hex(nonce)}_{counter}`). This rules out reuse across
   restart, across plugin respawn, and across `deliver_inbound`
   calls. Plugins therefore never need to evict tombstones to reclaim
   id-space, and the codec-layer discard and queue-purge checks are
   trivially safe.

   **Host-side emission rule.** The host MUST NOT emit new
   `stream_frame` or `inbound/stream_end` after writing the
   `abandon_inbound` response. The host enforces this with a
   per-`stream_id` boolean that the `abandon_inbound` handler sets
   *before* writing its response; the host's emitter consults the
   boolean under the same mutex that serializes writes to the stdio
   pipe, so no frame can race past the ACK out the wire. A host that
   emits after ACK is a protocol violation; the plugin SDK logs each
   such frame to `garyx doctor` and continues to discard them.

   These rules together mean the plugin observes exactly one
   terminal event per stream regardless of how bytes interleave,
   and the SDK's discard paths are both upstream (codec read) and
   downstream (queue execution) of the dispatch boundary.

   After `abandon_inbound.ok`, the SDK delivers a **single** terminal
   event to plugin user code on the per-stream serial queue: a locally
   synthesized `on_end` callback with `status: { "error":
   "stop_grace_expired" }`. This is NOT labelled `inbound/stream_end` — it's a
   distinct `on_stream_abandoned` callback in the SDK, so the two
   sources of terminality never collide. Plugin code MUST treat it as
   "do not ACK upstream" (same rule as host-shutdown cancellation,
   §7.1).

4. `stop` is idempotent. The `stop` RPC resolves only after every
   active stream has either observed `inbound/stream_end` OR had its
   `abandon_inbound` acknowledged by the host.

**Authority rule, unambiguous.** For any `stream_id`, exactly one of
three terminal events reaches plugin user code, and each of them
forbids the others:

- `inbound/stream_end` from host (the normal case) — authoritative; SDK
  closes the per-stream queue.
- `abandon_inbound` initiated by plugin, ACKed by host — host
  promises no more frames; SDK fires `on_stream_abandoned` instead
  of `on_end`.
- Host shutdown cancellation — host pre-emptively emits
  `inbound/stream_end` with `status: { "error": "host_shutting_down" }`
  during its own shutdown, which is just the first case with a
  specific status string.

There is no "plugin synthesizes an `inbound/stream_end`-shaped event" path
any more. That was the round-3 contradiction, and this replaces it
with a real protocol-level RPC (`abandon_inbound`) that makes the
host authoritative about stream lifecycle without tearing plugin
bookkeeping.

**Grace vs idle-timeout coordination.** The stop grace and the
per-stream idle timeout (§11.1, default 60s) are independent; the
plugin honors whichever fires first for a given stream. A healthy
long-lived stream is expected to hit the stop grace well before the
idle timeout — that is intentional. If the user wants long-running
agent tasks to survive a `stop`, the operator raises
`[runtime].stop_grace_ms`; the ceiling the host enforces is 60000ms
(matches the idle-frame timeout so the two concepts never drift).

Defaults:

- `[runtime].stop_grace_ms` — 5000
- `[runtime].shutdown_grace_ms` — 3000
- Host-enforced upper bound on `stop_grace_ms` — 60000

This is stricter than today's `MinoLabChannel::stop` (which just
calls `JoinHandle::abort`). The migration in §13 explicitly covers
retrofitting this drain behavior; "port verbatim" was wrong and is
corrected there.

### 6.3 `shutdown`

Host asks plugin to flush, close sockets, and exit within a **3-second
grace window**. After that the host `SIGTERM`s, waits 2s more, then
`SIGKILL`s.

### 6.3a `describe` (dry-run discovery)

Pre-migration and diagnostic path. Host can spawn a plugin with
`initialize.params.dry_run = true` and immediately call:

```jsonc
// request
{ "method": "describe", "params": {} }
// result
{
  "plugin": { "id": "minolab", "version": "0.2.0" },
  "protocol_versions": [1],
  "schema": { /* account config JSON Schema, §8.2 */ },
  "auth_flows": [ /* as declared in manifest */ ],
  "capabilities": { /* as declared in manifest */ }
}
```

In dry-run mode:

- `initialize.accounts` MUST be empty.
- Plugin MUST NOT open network sockets, start timers, or touch disk
  beyond what it needs to build the `describe` response.
- `start`, `stop`, `deliver_inbound`, `dispatch_outbound`,
  `auth_flow/*` all respond with `-32000 NotInitialized`.
- There is **no transition from dry-run to normal operation.** A
  dry-run process is terminated via `shutdown`; a real run is a
  *separate* `initialize` on a freshly-spawned child with
  `dry_run: false` (the default). Only `describe` and `shutdown` are
  valid in the dry-run process lifetime.

Rationale for a separate spawn rather than in-place promotion:

- The preflight (§13 step 4) runs **before** the host has constructed
  `ChannelPluginManager`, `MessageRouter`, or `MultiProviderBridge`;
  those structures are what `initialize` populates on a real run.
  Bootstrapping them just to tear them down would double-couple
  preflight to steady-state code.
- A clean re-spawn keeps the lifecycle state machine linear (the one
  drawn at the top of §6); no "dry_run → real" edge to reason about
  and no risk of a dry-run plugin carrying stale state into a real
  run.
- The cost is one extra process spawn per plugin during preflight
  (≈50–200ms on macOS/Linux for minolab-shaped binaries), paid once
  at host start. Acceptable.

This is the RPC the config-migration preflight (§13 step 4) relies on.
It's also what `garyx doctor` uses to introspect installed plugins
without starting them.

### 6.4 Account reload

**v1 rule: account edits force a full plugin respawn.** The host
calls `stop` (§6.2), terminates the child process, persists the new
config, then *spawns a fresh process* with the updated `accounts` in
`initialize`. This is not the same as the existing
`ChannelPluginManager::restart_plugin` (which only cycles
start/stop on the same in-memory plugin instance and does NOT re-run
`initialize` with new accounts) — respawn means "tear the process
down, launch a new child".

**Required host-side changes (must land with step 1 of §13).**
The existing reload surfaces are each incompatible and must be
updated:

1. `ChannelPluginManager::restart_plugin`
   (`garyx-channels/src/plugin.rs:257`) today only calls `stop` then
   `start`. For plugin-owned channels this must be extended — or
   joined by a new `respawn_plugin` method — that reconstructs the
   `SubprocessPlugin` with fresh accounts and re-runs `initialize`.
2. The CLI hot-reload path
   (`garyx/src/commands.rs:83,184`) already rebuilds the entire
   manager from scratch, so respawning plugins is automatic there;
   no change needed beyond plumbing the new accounts into
   `BuiltInPluginDiscoverer` / `ManifestPluginDiscoverer`.
3. The gateway `/api/settings/reload` path
   (`garyx-gateway/src/api.rs:1795` → `apply_runtime_config` in
   `composition/app_state.rs:97`) currently swaps config + dispatcher
   without touching plugins. It MUST be extended to:
     a. Compute the set of plugin-owned accounts whose *config*
        changed vs the previous snapshot.
     b. For each affected plugin id, call the new respawn entry on
        `ChannelPluginManager` with the new account list.
     c. Rebuild `ChannelDispatcherImpl` last, so outbound lookups
        only resolve once the respawned child has finished
        `initialize`.

Hot-reload without respawn (the `reload_accounts` RPC hinted at in
v1 drafts) is **deferred to v2** behind an explicit
`[capabilities].hot_reload_accounts` flag. When it lands, it will be
a two-phase RPC (`prepare_reload` → `commit_reload`) with explicit
rollback on prepare failure, not a best-effort diff. Reasons for the
deferral: diffing accounts while inbound streams are in flight is a
transactional problem (who owns `stream_id`s created under the old
config? are outbound dispatcher entries invalidated?), and a full
respawn is ~200ms for minolab-shaped plugins.

**Call-site consolidation.** Today's codebase has several entry
points for applying a new `GaryxConfig`:

| Entry point | File:line |
|---|---|
| `ChannelPluginManager::restart_plugin` | `garyx-channels/src/plugin.rs:257` |
| CLI hot-reload watcher | `garyx/src/commands.rs:83, 184` |
| Gateway `/api/settings/reload` | `garyx-gateway/src/api.rs:1795` |
| Gateway PUT `/api/settings` (direct apply) | `garyx-gateway/src/api.rs:1731–1733` |
| MCP-sync reload path | `garyx-gateway/src/mcp_config.rs:642–645` |

Rather than patch each call site, **all plugin-affecting reload
paths funnel through a single `AppState::apply_runtime_config`**
(`garyx-gateway/src/composition/app_state.rs:97`). The protocol change
is localised:

- `apply_runtime_config` gains a "plugin respawn step" that (a)
  diffs the previous and new configs, (b) for each
  plugin-owned-channel whose account set or config changed, calls
  `ChannelPluginManager::respawn_plugin(id, new_accounts)` — a new
  method distinct from `restart_plugin` that tears down the
  subprocess and launches a fresh one with fresh `initialize`
  arguments, (c) rebuilds `ChannelDispatcherImpl` last.
- CLI hot-reload and the gateway's `/api/settings/reload` both go
  through `apply_runtime_config`, so they inherit the respawn
  behavior for free.
- `ChannelPluginManager::restart_plugin` remains available for
  cycling a plugin without a config change (e.g. "restart after
  error" UX button). It does NOT re-run `initialize`. The two
  methods have different contracts and desktop surfaces them as
  distinct actions.

## 7. Data flow (plugin ↔ host)

### 7.1 Inbound: `plugin → host` (`deliver_inbound` + stream)

The subtle bit here is that `thread_id` is resolved by
`MessageRouter::route_and_dispatch` *early* (before most of the stream
is emitted), while today's `MinoLabChannel::build_response_callback`
only learns `thread_id` *after* `route_and_dispatch` returns — a race
that works in-process because the callback closes over a
`Arc<Mutex<Option<String>>>` holder. Over a pipe we need to be
explicit: **the host must hand the plugin `thread_id` before the first
stream event, not after the last one**.

The RPC is therefore split into a **short request** (get an
`InboundHandle` + `thread_id`) and a **server-initiated stream** (one
call to `inbound/stream_frame` per event, terminated by `inbound/stream_end`).

```jsonc
// 1. plugin → host, request
{
  "id": 42,
  "method": "deliver_inbound",
  "params": {
    "account_id": "product_ship",
    "from_id": "issue_123",
    "is_group": false,
    "thread_binding_key": "issue_123",
    "message": "hello",
    "run_id": "minolab-<uuid>",
    "reply_to_message_id": null,
    "images": [{ "data": "<base64>", "media_type": "image/png" }],
    "file_paths": ["/tmp/garyx/attachments/foo.pdf"],
    "extra_metadata": { "issue_id": "issue_123" }
  }
}

// 2. host → plugin, response — resolves as soon as the router has
//    assigned the thread and accepted the request for dispatch.
//    DOES NOT wait for agent completion.
{
  "id": 42,
  "result": {
    "stream_id": "str_7a2f…",       // server-assigned, unique per host process lifetime (§6.2)
    "thread_id": "thr_7f…",         // known BEFORE any stream frame
    "local_reply": null             // ALWAYS null in the shipped host; see below
  }
}

// 3. host → plugin, notifications — one per StreamEvent,
//    keyed by stream_id (NOT the request id).
{ "method": "inbound/stream_frame",
  "params": { "stream_id": "str_7a2f…",
              "seq": 0,
              "event": { "type": "delta", "text": "partial chunk" } } }

{ "method": "inbound/stream_frame",
  "params": { "stream_id": "str_7a2f…",
              "seq": 1,
              "event": { "type": "boundary", "kind": "user_ack" } } }

// 4. host → plugin, terminal notification — carries any tail state
//    (final thread_id, bookkeeping deltas) the plugin needs for
//    its ACK / persist step. The shipped host also attaches
//    `dispatch_metadata` (e.g. `{ "session_id": … }`) when it has
//    provider session attribution for the run.
{ "method": "inbound/stream_end",
  "params": { "stream_id": "str_7a2f…",
              "seq": 2,
              "status": "ok",
              "thread_id": "thr_7f…",
              "final_text": "full concatenated agent reply" } }
```

Design notes:

- **`thread_id` is delivered before any `stream_frame` is emitted.**
  Races between `Done` event arriving before the response are gone;
  the plugin always has the `thread_id` from step 2.
- **`stream_id` scopes the stream**, not the request id. The request
  id is free to be reused for other RPCs. This also opens the door to
  multiple concurrent inbound streams per plugin without numbering
  collisions.
- **`seq` is monotonic per `stream_id`**. JSON-RPC over a single
  stdio pipe is already ordered, so `seq` is a belt-and-suspenders
  correctness check (and survives any future move to a transport that
  reorders).
- **`inbound/stream_end.final_text`** avoids forcing the plugin to reimplement
  `merge_stream_text` / `apply_stream_boundary_text`. If the plugin
  only cares about the final reply (minolab's case), it can ignore all
  `stream_frame`s and read `final_text` from `inbound/stream_end`.
- **Cancellation**: if the host is shutting down mid-stream it emits
  `inbound/stream_end` with `status: { "error": "host_shutting_down" }`.
  Plugins should treat this as "do not ACK upstream"; see §11.
- **Shipped status values**: the shipped host emits
  `inbound/stream_end` only when the agent stream completes, always
  with `status: "ok"`. Pre-dispatch failures surface as the
  `deliver_inbound` JSON-RPC error response instead (no stream frames,
  no terminal). Plugins MUST NOT depend on receiving an error-status
  terminal today; treat the error-status shape as reserved.

**Per-stream completion rule (normative).** `inbound/stream_end` is the
**authoritative terminal event for a stream_id**. At the moment a
plugin processes `inbound/stream_end`:

- Any remaining `inbound/stream_frame` with the same `stream_id`
  already in the plugin's inbound pipe MUST be drained and applied
  to plugin-side accumulators **before** the plugin acts on
  `inbound/stream_end`. The SDK enforces this by dispatching per-`stream_id`
  frames on a *single* serial task and treating `inbound/stream_end` as just
  another ordered frame that happens to close the queue.
- `inbound/stream_end.final_text` is the source of truth for the final
  message body. A plugin that reconstructs text from `stream_frame`s
  itself MUST verify it matches `final_text` and use `final_text` on
  mismatch. Mismatches are a bug report to `garyx doctor`, not a
  protocol violation — streams can legitimately skip frames on host
  backpressure.
- A plugin that does persist-on-done work (minolab's `publish_message`
  + `record_outbound`) MUST do that work **inside** its `inbound/stream_end`
  handler, not inside a `stream_frame` handler, and MUST NOT access
  per-stream state (`thread_id`, accumulated text) after that
  handler returns. The SDK exposes this as `Stream::on_end(FnOnce)`.

Result: no nondeterministic ACK/persist behavior across plugin
implementations. Either the plugin follows the serial-per-stream
contract (SDK default) or it opts out explicitly and takes
responsibility for its own ordering.

The plugin is responsible for:

- uploading attachments and recording the channel-side message id,
- calling `record_outbound` (§7.3) if it wants the host's thread
  persistence layer to track the outbound message.

**Local replies (shipped behavior).** When the router answers a
request locally — native commands such as `/threads`, or any
synchronous-reply path — the shipped host does NOT put the text in the
`deliver_inbound` response: `result.local_reply` is always `null`, no
`inbound/stream_frame` or `inbound/stream_end` is emitted for that
request, and the reply is delivered through the plugin's own
`dispatch_outbound` (§7.2) tagged with the resolved `thread_id`. A
plugin therefore needs no special local-reply handling at all; its
ordinary outbound path receives the message.

### 7.1a Native stream-event generation (`dispatch_stream_event`)

Plugins that advertise `dispatch_stream_event: true` in their
capabilities (manifest and `initialize` response) opt into the second,
native streaming generation: instead of the §7.1
`inbound/stream_frame` / `inbound/stream_end` notifications, the host
sends each agent event as a `dispatch_stream_event` **request**
carrying the full stream envelope (`account_id`, `chat_id`,
`endpoint_identity`, `thread_id`, `run_id`, `event`, optional
`delivery_thread_id`), and the plugin renders the stream itself. This
is also the fanout path bound non-origin endpoints use. The §7.1
frame notifications remain the default for plugins without the
capability — both generations are permanent parts of this protocol,
not a migration window. Capability gating on the host side:
`outbound && !dispatch_stream_event` selects the host-rendered legacy
adapter for outbound fanout; `dispatch_stream_event` selects native
envelope delivery.

### 7.2 Outbound: `host → plugin` (`dispatch_outbound`)

For tool-initiated / scheduled / cross-channel messages the host calls:

```jsonc
{
  "method": "dispatch_outbound",
  "params": {
    "account_id": "product_ship",
    "chat_id": "issue_123",
    "delivery_target_type": "chat_id",
    "delivery_target_id": "issue_123",
    "text": "reminder: ...",
    "reply_to": null,
    "thread_id": null
  }
}
// result: { "message_ids": ["out_7a2..."] }
```

This is the `OutboundMessage` struct from `channels::dispatcher` minus
the `channel` field (redundant — the host already knows which plugin
it's talking to).

### 7.3 Record outbound (`plugin → host`, `record_outbound`)

Lets plugins persist outbound message ids into the host thread store
so replies get threaded correctly:

```jsonc
{
  "method": "record_outbound",
  "params": {
    "thread_id": "thr_7f...",
    "channel": "minolab",
    "account_id": "product_ship",
    "chat_id": "issue_123",
    "reply_to": null,
    "message_id": "out_7a2..."
  }
}
```

### 7.4 Host-provided HTTP fetch (optional, `plugin → host`, `fetch_resource`)

Some channels (minolab included) need to *download an attachment by
opaque token*, and that download is gated on a bearer token the plugin
already owns. Plugins should do this themselves — **the host does not
provide an HTTP client**. The surface area is zero here on purpose.

(If we find we do need shared retry/backoff or the host's cookie jar,
that's a v2 addition behind an additive capability flag.)

## 8. Configuration & the schema

### 8.1 Plugin manifest (`plugin.toml`)

```toml
[plugin]
id          = "minolab"
aliases     = ["mino"]
version     = "0.2.0"
display_name = "MinoLab"
description = "Proprietary Issue / AgentMessages channel."

[entry]
# Relative to the manifest. The host executes this as a child process.
binary = "./garyx-minolab-plugin"
# Optional env vars exposed to the child beyond the host baseline.
env = { RUST_LOG = "info" }

[capabilities]
outbound  = true
inbound   = true
streaming = true
images    = true
files     = true
# If false, host falls back to process restart on config edits.
hot_reload_accounts = true

# Account config schema — JSON Schema draft 2020-12 subset, see §8.2.
# Inlined here for manifest self-containment.
[schema]
"$schema"  = "https://json-schema.org/draft/2020-12/schema"
type       = "object"
required   = ["token", "base_url"]

[schema.properties.token]
type        = "string"
title       = "API Token"
format      = "password"             # UI hint: mask input
description = "Issue to personal access token from mino-lab admin."

[schema.properties.base_url]
type        = "string"
format      = "uri"
title       = "Base URL"
default     = "https://minolab.example.com"

[schema.properties.poll_interval_secs]
type        = "integer"
minimum     = 1
default     = 10
title       = "Poll interval (seconds)"

[schema.properties.workspace_dir]
type        = "string"
format      = "directory-path"       # UI hint: render DirectoryInput
title       = "Workspace directory"

# Optional: custom auth flows the plugin supports. Host exposes these
# as additional buttons on the account form (alongside plain field
# editing). See §8.3.
[[auth_flows]]
id      = "device_code"
label   = "One-click via device code"
prompt  = "We'll open a page in your browser and confirm here."
```

### 8.2 What subset of JSON Schema we support

**Scope.** The schema describes a **plugin-owned account's bootstrap
fields** — the stuff a user types into "Add bot": credentials, base
URLs, poll intervals, feature switches. It is **not** a replacement
for the full `FeishuAccount` / `TelegramAccount` typed models, which
carry routing policy (`groups: HashMap<…>`, `allow_from`, topic modes,
owner targets, etc.) and stay as Rust types in `garyx-models`.

Rationale: today's in-tree channels (`telegram` / `feishu` / `weixin`)
grew nested routing config because they are tightly integrated with
the host's routing policy. Plugin-owned channels, by construction, do
**not** have access to `MessageRouter` internals for policy decisions
— they deliver inbound and dispatch outbound, nothing else. Their
account shape stays much flatter, and the schema is sized for *that*
shape, not the built-ins' full feature matrix.

Concretely:

- Plugin accounts store their fields in `config: HashMap<String, Value>`
  (§9.3). Schema validates this payload.
- Built-in accounts keep their existing typed forms and existing
  hand-written React dialogs for now. If a built-in ever migrates to
  the plugin protocol (§13 step 5) the routing fields either move to
  a sibling `routing:` block governed by the host or are lifted into
  protocol-level policy.

Allowed schema vocabulary:

- `type`: `string` / `integer` / `number` / `boolean` / `array` / `object` / `null`
- `required`, `properties`, `additionalProperties`, `patternProperties`
- `items` (arrays), `enum`, `const`
- `minLength`, `maxLength`, `pattern`, `format`
- `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`
- `default`, `title`, `description`
- `oneOf` / `anyOf` for tagged-union configs (discriminator via `const`)
- `$ref` to sibling `$defs` entries (no remote refs — we will not
  dereference URLs)
- UI-hint `format` extensions (**garyx-specific**):
  - `"format": "password"` — masked input
  - `"format": "directory-path"` — DirectoryInput with native picker
  - `"format": "file-path"`
  - `"format": "uri"`
  - `"format": "duration-seconds"` — number spinner labelled "seconds"
- UI hints under `x-garyx`:
  - `x-garyx.widget`: `"select" | "textarea" | "segmented" | "tag-list"`
  - `x-garyx.placeholder`: string
  - `x-garyx.secret`: boolean — §8.4
  - `x-garyx.advanced`: boolean — hide under a collapsible section
  - `x-garyx.locked_after_create`: boolean — read-only in Edit dialog

Objects/maps/arrays are fully supported (e.g. `allow_from: string[]`
renders as a tag-list, `patternProperties` renders as an editable
key-value table). We explicitly reject:

- `if`/`then`/`else`, `dependentSchemas`, `contentSchema`,
  `contentEncoding`, `unevaluatedProperties` — they make the renderer
  non-compositional.
- `$ref` to external URIs.

**Anything outside this allow-list is ignored by the renderer and
flagged in `garyx doctor`.** We want a deterministic subset, not a race
between plugins and the UI.

### 8.3 Custom auth flows

Password/token is "just a field". But feishu/weixin need QR/device-code
dances, and future plugins will need OAuth. The state machine runs
entirely **inside the plugin**; the host is a dumb forwarder to the
desktop renderer.

> **§8.3 — Revised (April 2026).** The earlier revision of this
> section defined a 9-primitive display catalogue and an 8-state
> poll enum. Real implementation experience across feishu / weixin /
> minolab showed the protocol was carrying state-machine leakage
> (scanned / slow_down / awaiting_input / denied / expired / canceled /
> error) and display variants that never shipped (countdown / input /
> select / error / progress). The in-process
> [`AuthFlowExecutor`](../../garyx-channels/src/auth_flow.rs) trait
> and the subprocess RPC DTOs have been aligned to a much smaller
> surface. What follows is the shipped shape — three states, two
> display items, one start input.

#### The UI surface: `config_methods[]`

Every plugin's catalog entry carries a
`config_methods: Vec<ConfigMethod>` array listing the configuration
methods the channel supports, in the order the UI should render them:

```jsonc
"config_methods": [
  { "kind": "form" },       // render the JSON-Schema form
  { "kind": "auto_login" }  // render a "Sign in" button that drives
                            // the auth flow below
]
```

| `kind` | UI renders |
|---|---|
| `form` | The plugin's JSON Schema as an editable form. |
| `auto_login` | A button that kicks off the auth-flow RPCs below. On success, the returned `values` are merged back into the form so the user can still review before saving. |
| *anything else* | Forward-compat: a method a newer plugin declares that an older host doesn't recognise. UIs MUST render nothing for it. |

Built-in channels today:
- `telegram`: `[form]` — copy/paste bot token from @BotFather.
- `feishu` / `weixin`: `[form, auto_login]` — form is always available; auto-login is the fast path.

Subprocess plugins get `[form]` automatically (they all ship a
schema); they get `auto_login` added iff their manifest declares at
least one `[[auth_flows]]` block.

#### Display is a list of `Text` / `Qr`

Two primitives. No `link`, no `countdown`, no inline inputs. URLs
are just `Text`; the UI is free to auto-linkify if it wants. QR
payloads are just text too — the renderer draws the QR from the
string (native widget on the Mac App, terminal block characters in
the CLI). Forward-compat `Unknown` lets newer plugins ship new
kinds without breaking old hosts:

| Kind | Payload | Rendered as |
|---|---|---|
| `text` | `value: string` | Paragraph (UI MAY auto-linkify URLs) |
| `qr` | `value: string` | Inline QR image of `value` |
| *anything else* | — | Skipped |

Plugin-side convention: each item is independent — a plugin that
wants "instruction text + URL text + user code + QR" emits four
`Text`s in order followed by one `Qr`. Ordering is the plugin's
layout knob.

#### Poll state machine: `pending` / `confirmed` / `failed`

Channel-specific intermediate states (weixin's `scanned`, feishu's
`slow_down`, etc.) stay **inside the plugin** and collapse into
`pending`:

| Status | Terminal? | Semantics |
|---|---|---|
| `pending` | no | Keep polling. May carry a new `display` (atomic screen refresh — weixin's "already scanned, confirm on your phone") and/or a `next_interval_secs` bump (feishu's slow-down backoff). |
| `confirmed` | yes | Success. `values: {…}` is the partial account-config patch the UI merges into the form. |
| `failed` | yes | Terminal failure. `reason: string` is a plugin-authored message the UI MAY show verbatim. Denied / expired / unrecoverable-error all collapse to this with differing `reason`s. |

#### RPC shape

```jsonc
// host → plugin
{ "method": "auth_flow/start",
  "params": { "form_state": { /* whatever the user typed so far */ } } }

// → result
{ "session_id": "sess_abc",
  "display": [
    { "kind": "text", "value": "Open the link to authorise:" },
    { "kind": "text", "value": "https://accounts.feishu.cn/oauth/…" },
    { "kind": "text", "value": "Code: WXYZ-1234" },
    { "kind": "qr",   "value": "https://accounts.feishu.cn/oauth/…" }
  ],
  "expires_in_secs": 600,
  "poll_interval_secs": 5 }

{ "method": "auth_flow/poll",
  "params": { "session_id": "sess_abc" } }
// → { "status": "pending" }
// → { "status": "pending",
//     "display": [ { "kind": "text", "value": "Scanned, confirm on phone" } ],
//     "next_interval_secs": 2 }
// → { "status": "confirmed",
//     "values": { "app_id": "cli_…", "app_secret": "sec_…", "domain": "feishu" } }
// → { "status": "failed", "reason": "user denied the authorization" }

{ "method": "auth_flow/cancel",
  "params": { "session_id": "sess_abc" } }
// → {} (best-effort; plugin may ignore)
```

Notes:

- `form_state` is the only input to `start`. Everything the plugin
  might need — tenant selection, base URL, CLI-version tag — is
  carried here OR defaulted inside the plugin. The UI does not pick
  between `device_code` / `qr_code` / etc. — the plugin alone
  decides what running "auto login" means.
- Plugins are encouraged to default every field from their own
  JSON Schema (`schema.*.default`) so a pristine form can drive
  `auth_flow/start` immediately without any typing.
- `auth_flow/submit` is GONE. Multi-step input flows (if any plugin
  needs one in the future) belong on top of this protocol as a
  separate method, not inside `poll`.

#### Host-side plumbing

Built-in channels implement [`AuthFlowExecutor`](../../garyx-channels/src/auth_flow.rs)
directly (e.g. `FeishuAuthExecutor`). Subprocess plugins are
wrapped by [`SubprocessAuthFlowExecutor`](../../garyx-channels/src/plugin_host/auth_flow_bridge.rs),
which forwards the three RPC methods above through `PluginRpcClient`.
The gateway calls into either impl through the trait via
`ChannelPluginManager::auth_flow_executor(plugin_id)` — the Mac App
never knows whether it's talking to an in-process or subprocess
plugin.

HTTP endpoints (host → desktop, mirror of host → plugin):

- `POST /api/channels/plugins/{id}/auth_flow/start` — body carries
  `form_state`; response is the plugin's start response with an
  `ok: true` envelope and the session fields promoted to top-level
  keys (`session_id`, `display`, `expires_in_secs`,
  `poll_interval_secs`).
- `POST /api/channels/plugins/{id}/auth_flow/poll` — body carries
  `session_id`; response is the plugin's `AuthFlowPollResponse`
  JSON with `ok: true` added alongside the `status` / `display` /
  `values` / `reason` fields (gateway only mutates by injecting
  `ok`; all other fields are passed through unchanged).

Failure envelope for either endpoint:

```jsonc
{ "ok": false, "reason": "...", "message": "..." }
```

HTTP status maps from `AuthFlowError`:

| Status | When |
|---|---|
| 200 | Any successful `start` / `poll` result, including `failed` (that's an **expected** terminal state, not an HTTP failure). |
| 400 | `InvalidArgs` — the plugin rejected the `form_state` shape. |
| 404 | Plugin id unknown OR plugin advertises `config_methods` without `auto_login` (returns `reason: "no_auth_flow"`). Also: unknown `session_id` on poll (returns `reason: "poll_failed"`). |
| 500 | `Protocol` — plugin reply didn't conform to the DTO. |
| 502 | `Transport` — subprocess plugin unreachable / RPC timed out. |

Plugin ids accept aliases (`lark` → feishu).

### 8.4 Secret handling

Secret surface and defenses, per path:

1. **Config file at rest.**
   - Secrets live in `channels.plugins.<plugin_id>.accounts.<id>.config.<field>`
     inside the existing `garyx.toml` / equivalent. This is **plaintext
     by default** — same as today's `TelegramAccount.token`,
     `FeishuAccount.app_secret`, etc.
   - Defense: the config file is written with mode `0600` (unix) /
     owner-only ACL (win). The installer/bootstrap enforces this; the
     config loader refuses to read a file with broader perms and
     surfaces it in `garyx doctor`.
   - Explicit non-goal for v0.2: OS keychain integration. When it
     lands it will apply to both built-ins and plugins uniformly.

2. **Host log sink.**
   - Every field the plugin's schema marks `x-garyx.secret: true` is
     redacted before the host writes it to any sink (stdout, file,
     remote). Redaction keys are computed at schema-load time and
     applied both to outbound RPC payloads and to structured stderr
     records (see below).

3. **Plugin stderr.**
   - Plugins MUST emit **structured JSON per line** on stderr
     (`{ "level", "message", "fields": { … } }`). Host reads it,
     redacts fields named by the plugin's schema as secret, and
     rebroadcasts through `tracing`.
   - A plugin that writes non-JSON to stderr has its output captured
     into a single `tracing` event per line with NO redaction applied,
     and `garyx doctor` raises a warning (free-form stderr is a
     footgun; we want plugins to stop doing it).
   - The SDK (`garyx-plugin-sdk`) provides the default stderr writer so
     well-behaved plugins get this for free.

4. **Protocol transit.**
   - Stdio is parent/child only; no other process can see the bytes
     without `ptrace`-level access. We do not encrypt.
   - `auth_flow/poll` responses may contain secrets in `config_patch`;
     those are persisted immediately and never logged.

5. **Process listing / env.**
   - The host does NOT pass secrets via command-line args or
     environment variables. Everything arrives in `initialize`.
   - The plugin MUST NOT forward secrets to subprocesses it spawns
     (the SDK's default subprocess helper strips known-secret keys).

6. **Core dumps.**
   - On macOS/Linux the host disables core dumps for plugin children
     (`RLIMIT_CORE = 0`) by default, overridable via
     `GARYX_ALLOW_PLUGIN_COREDUMPS=1` for debugging.

Threat model covered: local non-privileged user on the same machine,
accidental logging, crash dumps shared with vendors. **Not** covered:
root-equivalent compromise, memory forensics, shared-hosting
multi-tenant boxes.

## 9. Host components

> **§9 — Revised (April 2026).** The architecture now has **one
> trait** (`ChannelPlugin`) that both built-in channels and
> subprocess plugins implement. The trait itself contains no
> subprocess-specific methods — spawn / exit / respawn are
> SubprocessPlugin implementation details and stay there. Callers
> (gateway HTTP handlers, CLI, Mac App) see a uniform surface
> regardless of where the plugin's code runs.

### 9.1 The `ChannelPlugin` trait

Lives in `garyx-channels/src/plugin.rs`. Method groups:

**Identity & metadata (every plugin)**
- `metadata() -> &PluginMetadata` — id, aliases, display name, version,
  description, `config_methods`.
- `capabilities() -> ManifestCapabilities` — outbound / inbound /
  streaming / images / files / delivery_model.
- `schema() -> Value` — JSON Schema describing one account's config.

**Channel-semantic lifecycle (`PluginLifecycle` super-trait)**
- `initialize`, `start`, `stop`, `cleanup` — take `&mut self`.
- Built-ins delegate to the wrapped `Channel` trait; subprocess
  plugins receive these as JSON-RPC calls driven by the manager.

**Auth flow**
- `auth_flow() -> Option<Arc<dyn AuthFlowExecutor>>` — returns an
  executor or `None` for form-only channels (Telegram). Built-in
  channels return concrete impls (`FeishuAuthExecutor`, etc.);
  subprocess plugins return a `SubprocessAuthFlowExecutor` bridge
  over the child's RPC.

**Outbound dispatch**
- `dispatch_outbound(OutboundMessage) -> SendMessageResult` —
  channel-blind send. Subprocess plugins forward over
  `PluginSenderHandle.dispatch()` end-to-end. Built-ins **currently
  still route through the legacy `SwappableDispatcher` map** — the
  trait method exists (`ManagedChannelPlugin` returns
  `Unsupported`), but the dispatcher's per-channel sender maps
  (`telegram_senders`, `feishu_senders`, `weixin_senders`) have
  not yet been unified.
- **Why this wasn't finished in the Scheme-A migration**: the
  clean path is to add an `async fn send(&self, msg)` method to
  the existing `Channel` trait and implement it on `TelegramChannel`
  / `FeishuChannel` / `WeixinChannel`. Each channel impl already
  has the HTTP client, tokens, and per-channel wire-format
  concerns (Telegram's numeric-id parsing, Feishu's reply-target
  encoding, Weixin's context_token retry loop) — they'd just move
  a few hundred lines each from the dispatcher into the channel
  where they architecturally belong. Until then, the dispatcher's
  `send_message` keeps branching by `channel` string and trait
  callers cannot dispatch to built-ins through `dispatch_outbound`.
  Subprocess plugins, which have no such legacy senders, work
  uniformly today.

**Account CRUD** (trait surface only; wire protocol pending)
- `list_accounts() -> Vec<AccountDescriptor>` — enumerate what the
  plugin knows about.
- `set_account(id, config)` / `remove_account(id)` — hot-reload
  entry points.
- **Important**: the trait methods exist today for in-process
  callers to use, but the matching JSON-RPC methods
  (`list_accounts` / `set_account` / `remove_account`) are NOT in
  the wire protocol (§5.3 inventory) and no reference subprocess
  plugin handles them. `SubprocessChannelPlugin` returns
  "unsupported — JSON-RPC method not in protocol" for all three
  until the wire contract catches up. Don't call these against a
  subprocess plugin from new code — add the RPCs to the spec +
  plugin(s) first.

### 9.1a Adapter structs

Two impls of the trait:

- **`ManagedChannelPlugin`** (`plugin.rs`) — wraps a built-in
  `Channel` + optional `Arc<dyn AuthFlowExecutor>` + fixed
  `capabilities` + `schema`. Constructed via `with_options` during
  `BuiltInPluginDiscoverer::discover`.
- **`SubprocessChannelPlugin`** (`plugin_host/subprocess_plugin.rs`)
  — wraps a `PluginSenderHandle` + an `Arc<Mutex<PluginRpcClient>>`
  so `respawn` can atomically swap the live client. Every trait
  method either returns manifest-captured data or forwards over
  the current RPC client.

Neither exposes the subprocess lifecycle (spawn / exit / respawn)
— those are methods of the raw `SubprocessPlugin` owned by
`ChannelPluginManager`, invisible to trait callers.

### 9.2 Plugin discovery

Replace `LocalDescriptorDiscoverer::from_env` with
`ManifestPluginDiscoverer` that scans:

1. `$GARYX_PLUGIN_DIR` (env override, colon-separated list on unix)
2. `~/.garyx/plugins/*/plugin.toml`
3. `<app resources>/plugins/*/plugin.toml` (bundled with desktop)

Each manifest becomes one `SubprocessPlugin`. Accounts for a plugin
come from `ChannelsConfig.plugins[plugin_id].accounts` in the existing
config file. In-process built-ins continue to use
`BuiltInPluginDiscoverer` unchanged.

### 9.3 `ChannelsConfig` changes

```toml
# existing: channels.telegram, channels.feishu, channels.weixin — unchanged.

# new: plugin-owned channels.
[channels.plugins.minolab.accounts.product_ship]
enabled = true
name    = "Product-Ship bot"
agent_id = "claude"
config = { token = "xxx", base_url = "https://minolab.example.com", poll_interval_secs = 10 }
```

`MinoLabConfig` / `MinoLabAccount` are **deleted** from
`garyx-models::config`. The host sees `HashMap<String, Value>` inside
`config` and forwards it to the plugin verbatim.

### 9.4 Outbound dispatcher

`ChannelDispatcherImpl` gains a plugin-backed sender entry for each
running `SubprocessPlugin`:

```rust
struct PluginSenderHandle {
    plugin_id: String,
    // Cloned from the SubprocessPlugin; dropping the handle does NOT
    // stop the subprocess. Handle holds an mpsc sender into the RPC
    // codec writer + a notification-routed response channel.
    rpc: PluginRpcClient,
    // Snapshot of capabilities so dispatcher can short-circuit
    // unsupported ops (e.g. file send against a text-only plugin).
    capabilities: PluginCapabilities,
}
```

Behavior:

- **Construction.** Every time `apply_runtime_config` rebuilds the
  dispatcher (§6.4), it walks the current `ChannelPluginManager` and
  creates one `PluginSenderHandle` per plugin whose state is
  `Running`. Handles are **not** registered for plugins in
  `Initializing` / `Error` — callers get
  `ChannelError::Config("plugin X not ready")` until the plugin
  actually reaches `Running`.
- **Respawn atomicity.** The dispatcher is stored as an
  `ArcSwap<ChannelDispatcherImpl>` (or equivalent atomic pointer):
  `send_message` loads the current `Arc` without locking and runs
  entirely against that snapshot. The swap itself happens under a
  short-lived synchronous lock that is **never held across an
  `.await`** — the respawn path builds the new
  `ChannelDispatcherImpl` while the old one is still live, acquires
  the swap mutex only long enough to publish the new `Arc`, then
  releases it. This avoids holding a mutex across RPC futures
  (deadlock risk) and guarantees no `send_message` ever observes a
  half-built dispatcher.

- **Quiesce order for in-flight outbound during respawn.**
  §6.4 says respawn "terminates the child process", which on its
  face races with any `dispatch_outbound` already awaiting a
  response on the OLD plugin RPC. The normative order is:

  1. Host publishes the NEW dispatcher via `ArcSwap::store`. From
     this instant, every *new* call resolves against the new
     handles.
  2. Host sends `stop` to the OLD child and waits up to
     `[runtime].stop_grace_ms` (§6.2 default 5000) for pre-existing
     `dispatch_outbound` requests to drain. The old
     `PluginSenderHandle` still holds the RPC writer, so pending
     calls complete normally during this window and their results
     return to their original callers. No new calls reach this
     handle because (1) already made it unreachable.
  3. If any `dispatch_outbound` is still pending at grace expiry,
     the host aborts their response futures and each caller
     receives `ChannelError::Connection("plugin X respawning;
     outbound aborted")`. Callers' existing retry policies apply
     and land on the new dispatcher from step 1.
  4. Host issues `shutdown`, then SIGTERM/SIGKILL per §6.3.

  This means "drains cleanly" is a **best-effort within
  `stop_grace_ms`**, not a guarantee of completion. The error is
  surfaced to the caller as a connection error, distinct from
  config errors, so the caller's retry behavior (which normally
  retries connection errors and not config errors) is correct.
  This also resolves the tension with §6.4: the child is
  terminated, but only after the drain window, and in-flight
  requests at that moment fail loudly rather than silently.
- **Routing.** `ChannelDispatcherImpl::send_message` matches on
  `channel` in this order:
  1. Built-in match (`telegram` / `feishu` / `weixin`) — unchanged.
  2. `self.plugin_senders.get(channel)` — routes via RPC.
  3. Otherwise `ChannelError::Config("unknown channel type")`.
- **Error mapping.** The plugin's `dispatch_outbound` RPC result
  maps to `ChannelDispatcher` semantics:
  - JSON-RPC `result` → `SendMessageResult`.
  - JSON-RPC `error` with code `-32601` (MethodNotFound),
    `-32602` (InvalidParams), `-32002` (AccountNotFound), or
    `-32007` (ChannelConfigRejected) → `ChannelError::Config(msg)`.
    These are caller-correctable errors; the supervisor takes no
    action.
  - JSON-RPC `error` with code `-32005` (ConfigRejected) is **not
    expected here** — it is a lifecycle-time error and the plugin
    should never emit it from `dispatch_outbound`. If observed, the
    host logs it as a plugin bug (`garyx doctor` advisory) and
    converts it to `ChannelError::Config(msg)` for the caller.
  - Any other `error` → `ChannelError::SendFailed(msg)`.
  - Host-side RPC timeout (§11.1) → `ChannelError::Connection("plugin X dispatch_outbound timed out")`.
  - Plugin process crash mid-request → caller receives
    `ChannelError::Connection("plugin X unavailable")`; the caller's
    retry policy applies, the supervisor restarts the child, and
    subsequent calls succeed once the handle is re-published.
- **Streaming callback for plugin-backed channels.**
  `build_streaming_callback` currently returns `Some` only for
  `telegram`. For plugin-backed channels, the host-side streaming
  contract is different: the AGENT streams back to the *plugin* via
  the `inbound/stream_frame` path (§7.1), and the plugin decides
  what to do with it. `build_streaming_callback` therefore returns
  `None` for plugin ids. If a future plugin wants the host to drive
  outbound streaming, we extend the protocol with an
  `outbound/stream_frame` pair, additive, guarded by a capability
  flag.

### 9.5 Desktop gateway surface

The gateway exposes three HTTP endpoints the desktop drives:

| Route | Method | Purpose |
|---|---|---|
| `/api/channels/plugins` | GET | Catalog of every plugin (built-in + subprocess). Returns `[{id, display_name, version, schema, capabilities, auth_flows, config_methods, accounts, icon_data_url?}]`. |
| `/api/channels/plugins/{id}/auth_flow/start` | POST | Passthrough to `auth_flow/start`. Body: `{form_state: {...}}`. |
| `/api/channels/plugins/{id}/auth_flow/poll` | POST | Passthrough to `auth_flow/poll`. Body: `{session_id}`. |

Each of the above is mirrored by an Electron IPC channel
(prefixed `garyx:`):

| IPC | Purpose |
|---|---|
| `garyx:fetch-channel-plugins` | renderer → main → HTTP GET. |
| `garyx:start-channel-auth-flow` | renderer → main → HTTP POST start. |
| `garyx:poll-channel-auth-flow` | renderer → main → HTTP POST poll. |

Legacy per-channel IPCs (`garyx:start-weixin-channel-auth`,
`garyx:start-feishu-channel-auth`, etc.) are still in place for the
existing `AddBotDialog` code path. They're scheduled for deletion
once every caller migrates to `garyx:start-channel-auth-flow`; the
protocol doesn't require them.

## 10. Desktop UI

> **§10 — Revised (April 2026).** Phase 3 of the desktop migration
> shipped a **channel-blind** React component tree that does NOT
> replace `AddBotDialog.tsx` in one sweep. The generic components
> (`AuthFlowDriver`, `PluginConfigPanel`) are ready to use and work
> end-to-end today; the old per-channel dialogs keep running in
> parallel until each channel's code gets ported.

### 10.1 Components shipped

- **`AuthFlowDriver`** (`channel-plugins/AuthFlowDriver.tsx`) —
  takes a plugin id + form state, calls `auth_flow/start`, renders
  the returned display list (Text / Qr items, auto-linkifies URL
  text), polls until Confirmed or Failed, invokes `onConfirmed`
  with the values patch. Zero channel-specific code.
- **`PluginConfigPanel`** (`channel-plugins/PluginConfigPanel.tsx`)
  — walks `config_methods[]` in order; renders `JsonSchemaForm`
  for `{kind:"form"}` and an AuthFlowDriver-backed button for
  `{kind:"auto_login"}`. Merges auto-login results into the form
  so the user can review before saving. Unknown kinds are
  forward-compat-skipped.
- **`JsonSchemaForm`** (existing) — JSON Schema 2020-12 subset
  renderer. Already used by `ChannelPluginCatalogPanel`.
- **`ChannelPluginCatalogPanel`** (existing) — diagnostic panel
  listing every channel from `/api/channels/plugins`. Not the
  "add a bot" entry point, but proves the catalog pipeline works.

### 10.2 Components still pending migration

- **`AddBotDialog.tsx`** (~583 lines) — still has per-channel
  branches and calls legacy `garyx:start-weixin-channel-auth` /
  `garyx:start-feishu-channel-auth` IPCs directly. Migration
  replaces the branches with `<PluginConfigPanel entry={…} />`.
- **`EditBotDialog.tsx`** (~424 lines) — same shape as AddBotDialog;
  migrates the same way, with `initialValue` pre-filled from the
  existing account config.
- Channel-specific preload methods (`startFeishuChannelAuth`,
  `pollFeishuChannelAuth`, etc.) are redundant now that
  `startChannelAuthFlow` / `pollChannelAuthFlow` exist; they
  disappear with AddBotDialog.

### 10.3 Copy / i18n

Labels, placeholders, descriptions all come from the schema
(`title`, `description`). Plugins ship zh-CN + en in parallel
fields (`title_i18n: { "en": "...", "zh-CN": "..." }`) — the
desktop picks based on `host.locale`. `PluginConfigPanel` itself
is currently zh-CN only; i18n of its "开始登录" / "保存" strings is
the one place copy needs to be maintained.

## 11. Crash handling, backpressure, and delivery guarantees

### 11.1 Crash detection and restart

- Child exits OR stdout EOF → host marks plugin `Error`, sets
  `last_error`, and schedules restart.
- **Malformed framing on stdout is a crash.** Plain `println!`
  output, corrupted `Content-Length`, or JSON that fails to parse as
  JSON-RPC: host logs the offending line, emits a `garyx doctor`
  advisory, and restarts the child. The SDK defends against this by
  redirecting Rust's default stdout to stderr so plugin authors
  cannot accidentally write to the wrong pipe.
- Backoff: 1s, 2s, 4s, 8s, 16s, 30s (cap). Reset to 1s after 5 minutes
  of healthy uptime.
- Per-RPC timeouts (host-enforced):
  - `initialize`, `start`, `stop`, `shutdown`: 10s
  - `auth_flow/*`: 15s
  - `dispatch_outbound`, `record_outbound`: 30s
  - `deliver_inbound`: **none** on the RPC itself (it returns fast),
    but each `inbound/stream_frame` has an **idle timeout of 60s**
    between frames. If the stream stalls the host emits `inbound/stream_end`
    with `status: { "error": "stream_idle_timeout" }` and invalidates
    the stream id; the plugin should not continue using it.
- Timeout expiry produces `-32603 InternalError` at the caller but
  does not by itself terminate the child; only crash or
  unresponsive-shutdown triggers process kill.

### 11.2 Backpressure and payload limits

Declared at the transport layer, enforced by both sides:

| Limit | Default | Override |
|---|---|---|
| Max JSON-RPC frame size | 8 MiB | `[runtime].max_frame_bytes` in manifest; host caps at 64 MiB regardless. |
| Max concurrent in-flight `deliver_inbound` per plugin | 32 | `[runtime].max_inflight_inbound`. |
| Max queued `inbound/stream_frame` per stream | 256 | Fixed; drop with `inbound/stream_end` error beyond. |
| Max inline image bytes per `deliver_inbound` | 4 MiB | Fixed. |

When a plugin exceeds in-flight inbound, the host responds to the 33rd
request with `{ code: -32006, message: "Busy" }`. The plugin should
queue/retry on its side (most pull channels do this naturally via
unacked messages).

**Normative rule for large media.** The host does not buffer beyond
the transport pipe. Plugins delivering inbound messages with media
MUST:

1. Write large assets (anything over ~1 MiB, always for files, always
   for images that aren't already ≤4 MiB compressed) to the shared
   temp directory and pass **paths** in `file_paths`.
2. Reserve inline `images[].data` base64 for small thumbnails / tiny
   assets the user is expected to see rendered immediately.
3. Compute the total frame size **before sending** and, if it would
   exceed `max_frame_bytes`, split into multiple `deliver_inbound`
   calls or spill more attachments to disk. The frame limit is a
   hard boundary — the host rejects with `-32008 PayloadTooLarge`
   and does not retry.

The shared temp directory is provided by the host in `initialize.host.data_dir`
under `attachments/inbound/` and is garbage-collected by the host 24h
after `inbound/stream_end`. Plugins do not need to clean up themselves.

This means typical phone-photo batches (every photo 3-8 MiB) flow
through `file_paths`, never through inline `images[]`. The SDK's
default `Host::deliver_inbound` helper does the spill automatically
based on declared media size.

### 11.3 Delivery guarantees (scoped honestly)

Delivery guarantees **depend on the upstream channel's ACK model.**
Three cases, each declared explicitly in the manifest via
`[capabilities].delivery_model`:

1. **`pull_explicit_ack`** (e.g. minolab's `poll` + `/ack`). The
   plugin holds the message in an "unacked" state upstream and only
   calls the upstream ACK API after `deliver_inbound` has returned
   AND `inbound/stream_end` reported `status: "ok"`. A host crash
   mid-processing means the message re-appears on the next poll.
   No host-side buffering needed. **Strongest guarantee.**

2. **`push_negative_ack`** (e.g. Feishu webhooks: upstream retries
   unless we 200 back within a short window). **The plugin owns its
   own HTTP ingress** — it listens on its own port, decodes the
   request, verifies the upstream signature, and is the one that
   eventually sends the 200 back. The host is not an HTTP
   intermediary in v0.2 (see §12.6). The plugin's crash-recovery
   contract is therefore:
   - Hold the upstream HTTP response open (do not 200 on receipt)
     until `inbound/stream_end.ok` arrives. Then 200.
   - If the pipe to the host is broken (host crashed), return a 5xx
     so upstream redelivers when the host is back.
   - De-duplicate on upstream `event_id` across redeliveries; after
     a crash-retry the same event may arrive twice.
   - Surface a `duplicate_suppressed` metric so operators can watch
     for retry storms.

   **Ingress URL registration.** The host has no way to guess the
   plugin's listener address (the plugin picks its port, potentially
   `0` for ephemeral). After `start` succeeds, a plugin with
   `delivery_model = "push_negative_ack"` MUST send:

   ```jsonc
   // plugin → host, notification
   { "method": "register_ingress",
     "params": {
       "account_id": "product_ship",
       "public_url": "https://mycorp.example.com/feishu/webhook",
       "local_url":  "http://127.0.0.1:48213/feishu/webhook"
     } }
   ```

   The host records both URLs per account and surfaces them in
   `garyx doctor` + desktop's account-detail view. They are advisory
   — the host does not poke the upstream service. **Registering the
   URL with the upstream provider (Feishu admin console, Telegram
   `setWebhook`, etc.) is either done manually by the user during
   setup (the desktop renders the URL with a copy button), or
   performed by the plugin itself via the provider's own API — not
   the host's responsibility.** When `host.public_url` is empty the
   plugin SHOULD set only `local_url` and let desktop warn the user.

   Middle-strength: guaranteed delivery modulo plugin de-dup bugs.
   Upgrade path to host-proxied ingress is additive (§12.6) — when
   it lands, plugins opt in by declaring
   `[capabilities].needs_host_ingress = true` and stop owning their
   own listener *and* stop emitting `register_ingress`.

3. **`push_at_most_once`** (ephemeral websocket frames, fire-and-
   forget push notifications where upstream has no retry). A crash
   between the plugin reading the frame and `inbound/stream_end.ok` drops
   the message silently. Plugins MUST advertise this so desktop can
   badge the channel with a "best-effort delivery" indicator and
   docs can warn users.

The manifest requires a single explicit value:

```toml
[capabilities]
delivery_model = "pull_explicit_ack"   # or push_negative_ack, push_at_most_once
```

A host-side durable WAL (`inbound.wal`) would let us promote case 3
to case 1 across the board. It's non-trivial (coordinating WAL ACK
with router side effects) and is deferred; see §15.4.

Minolab is `pull_explicit_ack`, so for the v0.2 shipping target the
strong guarantee holds.

## 12. Rejected alternatives

### 12.1 Dynamic loading (`libloading`, `dlopen`)

- Rust has no stable ABI; a plugin compiled against garyx 0.1.3 would
  UB on 0.1.4 unless every type crossing the boundary is
  `#[repr(C)]`. The tradeoff (a mountain of unsafe FFI types) is
  worse than a pipe.
- One plugin corrupting the process heap takes everything else down.

### 12.2 WASM / WASI plugins

- Perfect for sandboxing, but WASI's socket/HTTP story is still
  patchy. `minolab` already works fine; we don't want to rewrite it
  against WASI's "pre-open" model.
- We don't need sandboxing (§1 non-goals).

### 12.3 gRPC

- Codegen pipeline for a private-tree proto that has exactly one
  producer and one consumer is overkill.
- Localhost gRPC means either TLS headaches or accepting a plaintext
  port. Stdio skips both.

### 12.4 "Just make minolab a runtime-loaded Cargo feature"

- This was the motivation for rejecting **scheme A**. It keeps the
  channel in the garyx build graph → it must be open-source to let us
  open-source the repo → fails the stated goal.

### 12.5 REST/webhook plugins (plugin exposes HTTP, host POSTs)

- Requires every plugin to deal with auth, ports, and TLS.
- Breaks when users are behind corp firewalls.

### 12.6 Host-proxied ingress (host accepts webhook, forwards to plugin)

**Considered and deferred, not rejected.** The host already owns
`public_url`; it could expose `/plugin/<id>/webhook` and forward
inbound HTTP to the plugin over the RPC pipe. This would genuinely
help push-channel durability (§11.3 case 2) because the host could
WAL the HTTP body before ACKing upstream.

Why not in v0.2:

- v0.2's first plugin (minolab) is pull-based; we don't need it yet.
- Designing the forwarder correctly means decisions about TLS
  termination, path namespacing, signature verification (telegram's
  `X-Telegram-Bot-Api-Secret-Token`, feishu's `Verification-Token`),
  and request body size limits — none of which are trivial to get
  right.
- A v2 addition of ingress proxying is **additive**: the plugin gains
  a new RPC `inbound/http_delivered` and a new manifest capability
  `[capabilities].needs_host_ingress = true`. No breaking change.

Revisit when the first push-based third-party plugin lands.

## 13. Migration plan

Staged so that each step is independently shippable. "Shippable" here
means: the build is green, existing users see no regression, and the
step can be merged to `main` without finishing the next step.

### Step 1 — Protocol scaffolding (host side only)

Implement:

- `SubprocessPlugin`, `ManifestPluginDiscoverer`, JSON-RPC codec, and
  the supervisor (crash detect, backoff, structured stderr).
- `ChannelsConfig.plugins.<id>.accounts.<name>.config: Value` payload
  support.
- Outbound dispatch routing (§9.4) that resolves plugin-backed
  channels via RPC.
- Contract tests using a Rust dummy plugin that exercises every RPC
  in both success and crash paths (§14).

**Does NOT ship user-visible changes.** Built-ins still register via
`BuiltInPluginDiscoverer`; desktop dialogs are unchanged;
`AddBotDialog` still hand-codes `minolab`. `ManifestPluginDiscoverer`
finds no plugins unless the user sets `$GARYX_PLUGIN_DIR`.

This step explicitly does **not** claim to "unlock open-sourcing";
that only happens after step 4 when proprietary code is actually gone
from the tree.

### Step 2 — Descriptor bridge for the desktop UI

Add the generic renderer (`<ChannelForm>`, `<AuthFlowRunner>`) and
the generic IPC channels (`list-channel-descriptors`, the generic
auth-flow start/poll/submit/cancel). For now both the old hand-coded
dialogs AND the generic renderer are present; channels flag
themselves as "schema-capable" or "legacy" and desktop picks the
appropriate path.

Built-ins remain `legacy` (no descriptor emitted). No
user-visible change.

### Step 3 — Extract minolab

- New private repo / workspace member producing
  `garyx-minolab-plugin` binary.
- Port the poll loop, attachment handling, and response callback to
  use `garyx-plugin-sdk` (§Appendix A).
- **Retrofit `stop` drain semantics** (§6.2) while porting — this
  corrects the earlier "port verbatim" instruction, which was wrong
  because today's `JoinHandle::abort` does not honor drain.
- Publish a `minolab` manifest with schema + config schema marks the
  three fields (token, base_url, poll_interval_secs).
- End-to-end test: desktop points at the binary, adds an account via
  the generic dialog, inbound round-trips.

### Step 4 — Delete the built-in minolab (transactional)

- Remove `MinoLabChannel`, `MinoLabConfig`, `MinoLabAccount` from
  `garyx-models` and `garyx-channels`.
- Remove the hardcoded `minolab` branches from `AddBotDialog` /
  `EditBotDialog` / `channel-logo` / contracts.

**Config upgrade protocol (transactional, rollback-safe).** The
config file rewrite must survive three failure modes: process crash
mid-write, plugin not installed, plugin installed but its current
schema rejects the migrated shape (schema skew).

**Bootstrapping order (normative).** The preflight runs as a
*discrete, earlier phase* of host startup. The dependency is:

```
boot → preflight (this protocol) → apply_runtime_config → ChannelPluginManager::new
                     │
                     └── uses a lightweight ManifestOnlyDiscoverer +
                         a one-shot DryRunPluginLauncher; does NOT
                         construct ChannelPluginManager,
                         MultiProviderBridge, or MessageRouter
```

Concretely:

- A new `garyx-channels::preflight` module exposes
  `preflight_migrations(config_path, plugin_dirs) -> Result<(), PreflightError>`.
- It uses a `ManifestOnlyDiscoverer` — a code path that reads and
  validates `plugin.toml` files without instantiating
  `SubprocessPlugin` — and a `DryRunPluginLauncher` that spawns one
  child per needed plugin, drives `initialize { dry_run: true }`
  followed by `describe` followed by `shutdown`, then joins.
- Steady-state `ManifestPluginDiscoverer` + `ChannelPluginManager`
  are only constructed **after** preflight returns `Ok`. A failed
  preflight aborts the host process with a non-zero exit.

This decoupling means the preflight cannot accidentally depend on
runtime state that hasn't been built yet, and upgrading the
`ChannelPluginManager` API never forces changes to the migration
path.

1. **Preflight (host start, before any network / plugin I/O).**
   - Load the existing `garyx.toml`. If it contains no
     `channels.minolab` block, skip to "done".
   - Locate the installed `minolab` plugin via
     `ManifestOnlyDiscoverer`. If missing, abort preflight with a
     clear error ("Install the minolab plugin from <URL> or remove
     `channels.minolab` from garyx.toml"), do NOT migrate, do NOT
     start.
   - Launch the plugin via `DryRunPluginLauncher` (separate child:
     `initialize { dry_run: true, accounts: [] }`, then `describe`,
     then `shutdown`). Compare shapes: built-in fields (`token,
     base_url, poll_interval_secs, enabled, name, agent_id,
     workspace_dir`) must all appear in the new schema with
     compatible types.
   - If the schema is incompatible (schema skew), abort with
     actionable error: "The installed minolab plugin vN.N.N does not
     accept field `X` from your current config; upgrade the plugin or
     roll back garyx."

2. **Atomic write.**
   - Build the new `garyx.toml` in memory.
   - Write to `garyx.toml.next` in the same directory (same device
     → atomic rename).
   - `fsync` the file.
   - Rename `garyx.toml` → `garyx.toml.backup-<ISO8601>`.
   - Rename `garyx.toml.next` → `garyx.toml`.
   - `fsync` the directory.
   - Any failure before the final rename aborts with the old file
     intact; any failure after means both old-backup and new file
     exist and the host can verify on next start.

3. **Verify.**
   - Re-read `garyx.toml`, confirm the migrated section round-trips.
   - Emit a migration notice ("Migrated minolab config to plugin
     format; backup saved at X.") via the normal startup log and
     the desktop notification channel.

4. **Rollback.**
   - If anything in step 3 fails, rename the backup back over
     `garyx.toml` and exit with a non-zero status. Do not leave a
     half-migrated config in place.

5. **Re-run safety.**
   - The upgrader is idempotent: if `channels.plugins.minolab`
     already exists and `channels.minolab` does too, the upgrader
     refuses to auto-merge and asks the operator (because the two
     could disagree — safer to surface than silently pick one).

**After step 4 the garyx repo is open-sourceable** — no proprietary
channel code remains.

### Step 5 — Optional: built-ins to the protocol

- Revisit after step 4 lands. Only if third parties start shipping
  plugins and faster built-in release cycles become a concrete ask.
- Routing config (`groups`, `allow_from`, policy fields) stays as
  typed models in `garyx-models`; only the transport/credential
  surface migrates. If that decoupling proves too awkward we abandon
  the step.

## 14. Testing strategy

Three layers, each with explicit must-have cases:

### 14.1 Unit — in-memory transport

- Mock host and plugin talking over `tokio::io::duplex()`. No
  subprocess. Fast, deterministic. Covers the golden path for every
  RPC and the state machine of `auth_flow`.

### 14.2 Codec fuzzing & malformed-input tests

Explicit, required cases (not optional):

- **Stdout pollution:** dummy plugin emits `println!("hello\n")`
  before its first JSON-RPC message → host detects malformed framing
  → child restarted → `garyx doctor` advisory recorded.

  **The host's parse-fail-and-restart behavior is the canonical
  defense against stdout pollution, not the SDK's stdout redirect.**
  The SDK redirect helps Rust plugins that use the SDK, but we must
  assume future plugins in Python / Go / third-party-crate-using Rust
  will emit stray bytes; the protocol has to tolerate it deterministically
  instead of relying on well-behaved authors. Every release runs this
  test against a purposefully-misbehaving plugin.
- **Chunked frame boundaries:** host assembles a frame across
  multiple read() calls (split in the middle of `Content-Length`,
  split in the middle of the body). Passes.
- **Oversized frame:** plugin emits `Content-Length: 100000000` →
  host rejects before allocating, restarts child.
- **Interleaved in-flight requests:** host sends 32
  `deliver_inbound`-triggered `dispatch_outbound`s concurrently,
  plugin responds out of order, all resolve on the right ids.
- **Concurrent notifications + responses:** plugin emits
  `inbound/stream_frame` for stream A while still writing the
  response to request B. Both sides parse correctly.

### 14.3 Integration — real subprocess

- `garyx-channels/tests/plugin_harness.rs` spawns a shipped-in-tree
  `tests/fixtures/echo-plugin` binary.
- Cases:
  - **Graceful shutdown** of a plugin with an in-flight stream —
    stream drains within the configured grace window.
  - **Crash during stream** — host invalidates the stream,
    supervisor restarts with backoff, next inbound works.
  - **Auth flow full cycle** — start → poll (`pending`) → poll
    (`slow_down`) → `awaiting_input` → submit → confirmed; desktop
    renderer snapshot stays stable.
  - **Config hot path** — edit an account in desktop → host stops
    the plugin, restarts with new config, first inbound after
    restart succeeds.
- **Malformed stderr** — plugin writes `not json\n` on stderr → host
  logs it as a single tracing event, keeps running.

### 14.4 Contract snapshot

Every schema in every bundled manifest is round-tripped through the
`<ChannelForm>` renderer in Vitest. Snapshot diff protects plugins
against renderer regressions we can't see the source of.

## 15. Follow-ups and resolved decisions

Nothing in this section blocks v0.2 implementation. The first three
items have resolved proposals that are part of v0.2; item 4 is
explicitly deferred.

1. **Versioning contract.** Resolved. Protocol versions are an
   integer that bumps on breaking changes (new required RPC, removed
   field). Host and plugin each declare the set of supported
   versions; the handshake picks the highest overlap. Additive
   features ride on `[capabilities]` flags inside `initialize`
   response, not on the version number. `garyx doctor` surfaces the
   effective version.
2. **Gateway-URL-required plugins.** Resolved. `requires_public_url:
   bool` in `[capabilities]`, checked at `initialize` and surfaced
   in desktop with a clear "Set up tunnel first" gate. Telegram /
   feishu set it true; minolab sets it false.
3. **Plugin self-update.** Out of scope for v0.2. Users update
   plugins the same way they installed them (drop binary into
   `~/.garyx/plugins/…`). `garyx doctor` warns on version mismatch
   against manifest `min_host_version`.
4. **Host-side durable inbox** for push channels (§11.3 case 2).
   **Deferred to v2 — explicitly not a v0.2 blocker.** v0.2 ships
   with the three-model contract in §11.3 and
   `push_negative_ack` plugins own their own HTTP listener. A host
   inbox (WAL-before-ACK) would be additive and is tracked as a
   capability flag (`[capabilities].needs_host_ingress`, §12.6) so
   that when it lands, existing plugins opt in without a protocol
   break. We will not design it until the first push-based
   third-party plugin concretely asks for it.

---

## Appendix A — Minimum viable plugin (skeleton)

```rust
// garyx-plugin-minolab/src/main.rs
use garyx_plugin_sdk::{Plugin, Host, InboundRequest, StreamEvent};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let host = Host::from_stdio().await?;  // blocks on `initialize`
    let plugin = MinoLabPlugin::new(host.accounts());
    host.run(plugin).await
}

struct MinoLabPlugin { /* as-is from today's MinoLabChannel */ }

#[async_trait]
impl Plugin for MinoLabPlugin {
    async fn start(&self, host: Host) -> anyhow::Result<()> {
        for account in self.accounts.iter().filter(|a| a.enabled) {
            let host = host.clone();
            let account = account.clone();
            tokio::spawn(poll_loop(account, host));
        }
        Ok(())
    }
    async fn stop(&self) -> anyhow::Result<()> { /* drain tasks */ }
    async fn dispatch_outbound(&self, ...) -> anyhow::Result<Vec<String>> { ... }
    async fn auth_flow_start(&self, ...) -> anyhow::Result<AuthFlowDisplay> { ... }
    async fn auth_flow_poll(&self, ...) -> anyhow::Result<AuthFlowResult> { ... }
}

async fn poll_loop(account: Account, host: Host) -> anyhow::Result<()> {
    loop {
        let messages = fetch_messages(&account).await?;
        for msg in messages {
            let req = InboundRequest { /* from msg */ };
            let (_result, stream) = host.deliver_inbound(req).await?;
            tokio::spawn(publish_stream_as_replies(account.clone(), stream));
        }
        tokio::time::sleep(account.poll_interval).await;
    }
}
```

The new crate `garyx-plugin-sdk` (shipped from the public garyx repo,
under MIT) is the *only* garyx-owned thing a plugin depends on.
