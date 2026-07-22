# Grok Build ACP Provider

## Goal

Garyx runs the installed Grok Build CLI as a native provider by launching
`grok --no-auto-update [--max-turns N] agent --always-approve --no-leader
stdio` and speaking Agent Client Protocol (ACP) version 1 over newline-delimited
JSON-RPC. There is no HTTP compatibility service between Garyx and Grok.

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
- Grok ACP does not expose a native session-fork operation. Garyx rejects a
  requested SDK session fork with a session error instead of silently starting
  an empty conversation.
- Configured model and reasoning defaults are applied with `session/set_model`
  after new/load, so the same behavior applies to fresh and resumed sessions.
- A positive configured `max_turns` is passed to the ordinary Grok process as
  its documented top-level `--max-turns` option.
- `session/update` notifications become assistant deltas and normalized tool
  start/result events. If the initial tool frame omits `rawInput`, the adapter
  waits for the first update and emits one complete append-only ToolUse row.
  Thought chunks remain provider-internal.
- The prompt timeout is an inactivity timeout and is refreshed by every ACP
  frame. Initialize, authenticate, session, and model requests retain fixed
  one-shot deadlines.
- Cancellation sends the ACP `session/cancel` notification when a session is
  known. The adapter waits briefly for both the flushed frame and ACP prompt
  settlement before the caller may drop the run. An acknowledged cancellation
  returns a normal partial result with accumulated text, messages, usage, and
  the native session ID; it is not persisted as a provider failure. Dropping a
  run after the bounded settlement window still terminates its isolated child
  process, while never issuing a session-delete operation, so the native
  session remains resumable.
- A dedicated stdout reader owns line accumulation. Selecting between incoming
  messages and cancellation cannot discard a partial JSON-RPC frame.

## Errors and discovery

Grok's structured JSON-RPC rate-limit code (`-32003`), typed error data, and
upstream HTTP status data (`429`, `503`, or `529`) map into Garyx rate-limit
state; ordinary error text is not pattern-matched. Model discovery performs ACP
initialize and reads Grok's advertised model state, including reasoning-effort
choices. Unknown notification fields and ACP extensions are retained or ignored
at this boundary rather than making the provider brittle.

ACP `end_turn` is a successful completion. `cancelled` is a successful partial
completion with an explicit stop indication. Known incomplete terminal reasons
(`max_tokens`, `max_turn_requests`, and `refusal`) return a soft unsuccessful
`ProviderRunResult`, preserving the transcript and native session rather than
becoming a hard transport failure. Unknown future stop reasons remain successful
but are surfaced in the existing result error text so they are not silently
discarded.

## Scoped decisions after review

- Native fork support is deferred until Grok exposes an ACP fork operation;
  rejection is deliberate and test-pinned, so there is no context-loss fallback.
- Garyx keeps using the existing `ProviderRunResult.success/error` contract for
  stop presentation instead of adding a provider-specific persisted stop field.
- Tool input is not re-emitted as a duplicate ToolUse because Garyx stream rows
  are append-only. Delaying the first row by one ACP update preserves one
  authoritative tool entry.
- Cancellation settlement is bounded. A non-responsive child may still be
  killed after the grace window; waiting indefinitely would make Stop and
  shutdown unbounded.
- The account-selection and credential-management surface remains entirely out
  of scope. This provider launches one ordinary Grok binary with the existing
  generic provider environment and never retries another identity.

## Product surface

`grok_build` is a first-class `ProviderType` with the built-in agent name
`Grok`, shared branded avatar assets, CLI/config aliases, gateway model catalog,
and matching Mac and iOS settings/identity labels. The Mac information
architecture remains canonical; mobile mirrors it with its existing native
provider-settings composition.
