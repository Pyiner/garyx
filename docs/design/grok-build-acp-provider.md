# Grok Build ACP Provider

## Goal

Garyx runs the installed Grok Build CLI as a native provider by launching
`grok --no-auto-update agent --always-approve --no-leader stdio` and speaking
Agent Client Protocol (ACP) version 1 over newline-delimited JSON-RPC. There is
no HTTP compatibility service between Garyx and Grok.

## Boundary

A small `grok-agent-sdk` crate owns process launch, ACP framing, initialization,
authentication, session requests, model discovery, cancellation, and structured
RPC errors. `garyx-bridge` owns Garyx lifecycle concerns and maps typed SDK
events into provider stream events. The gateway and clients consume the normal
provider, model-catalog, agent-identity, and settings contracts.

Each top-level run copies the existing generic provider `env` map into its SDK
launch configuration. The transport only applies that immutable per-run copy to
the child process and uses Grok's first advertised ACP authentication method.
It never reads, rotates, rewrites, persists, pools, or retries credentials, and
it contains no account-selection or credential-failover state.

## Session and stream lifecycle

- A new thread uses `session/new`; a resumed thread uses `session/load` with the
  exact persisted ACP `sessionId`. Garyx emits that native ID through
  `SessionBound` and stores it in the existing provider-scoped session field.
- Configured model and reasoning defaults are applied with `session/set_model`
  after new/load, so the same behavior applies to fresh and resumed sessions.
- `session/update` notifications become assistant deltas and normalized tool
  start/result events. Thought chunks remain provider-internal.
- Cancellation sends the ACP `session/cancel` notification when a session is
  known. Dropping a run also terminates its isolated child process, while never
  issuing a session-delete operation, so the native session remains resumable.
- A dedicated stdout reader owns line accumulation. Selecting between incoming
  messages and cancellation cannot discard a partial JSON-RPC frame.

## Errors and discovery

Grok's structured JSON-RPC rate-limit code (`-32003`), typed error data, and
upstream HTTP status data (`429`, `503`, or `529`) map into Garyx rate-limit
state; ordinary error text is not pattern-matched. Model discovery performs ACP
initialize and reads Grok's advertised model state, including reasoning-effort
choices. Unknown notification fields and ACP extensions are retained or ignored
at this boundary rather than making the provider brittle.

## Product surface

`grok_build` is a first-class `ProviderType` with the built-in agent name
`Grok`, shared branded avatar assets, CLI/config aliases, gateway model catalog,
and matching Mac and iOS settings/identity labels. The Mac information
architecture remains canonical; mobile mirrors it with its existing native
provider-settings composition.
