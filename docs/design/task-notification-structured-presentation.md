# Task Notification Structured Presentation

Status: revision 6 for adversarial review
(r1 3B+8M; r2 1B+7M+3m; r3 2B+5M+2m; r4 3B+3M+1m; r5 3B+1M — confirmed
and frozen through r5: root cause & principles, six-variant producer
inventory, single-PendingUserInput-constructor convergence, flattened
presentation payload, all client/UI decisions, prose-parser deletion,
range-rewrite skeleton, no-retroactive-trust, sdk_session_fork scrub +
fork re-homing, capability-block precondition, stream schema negotiation
+ cache invalidation, evidence attribution, rollout/TestFlight contract)
Owner: Gary (orchestrator thread)
Scope: garyx-models, garyx-router, garyx-bridge, garyx-gateway, garyx
(plugin host), garyx-channels, desktop renderer, iOS

## 1. Problem

(unchanged; confirmed) Busy-thread queue allowlist drops notification
semantics (73/252 live records unmarked → raw XML on both clients); four
client prose parsers coupled to template wording; provider runtime
secrets already leak into committed transcripts; semantics, runtime
config, and external input share one untyped map guarded by drifting
string filters.

## 2. Design principles

1. **The write path is the root of trust — not the data shape.** A
   committed record's provenance is trustworthy because every path that
   can write records either originates in an internal dispatcher (which
   alone supplies provenance) or is an untrusted-input path that
   structurally cannot carry it. Serde types do not authenticate anything;
   the type system exists to make the honest path the only compiling path
   for *our* code, and input-path separation handles *data*.
2. **Runtime configuration is typed and never persisted.** External
   runtime overrides are explicit per-ingress typed fields matching
   today's actually-consumed surface — no expansion, no silent removal of
   shipped product capability.
3. **Structure is never round-tripped through prose.**
4. **No retroactive trust** for history, imports, or raw-value writes.

## 3. Changes

### 3.1 Provenance: input-path separation + controlled store writes

#### Trust model (r5 BLOCKER 1 — replaces the unimplementable
"gateway-only capability" claim)

Rust has no friend crates, and `Deserialize` cannot be visibility-scoped,
so no serde type can *itself* prove origin. The enforceable layers are:

1. **Construction API split.** `garyx-models` defines
   `ProvenanceRecord` (the wire/record value: six variants, internally
   tagged `"kind"`, public read accessors, `Serialize`+`Deserialize` — it
   is data, not a credential) and `DispatchMetadata` with exactly two
   constructors: `DispatchMetadata::external(…)` — no provenance
   parameter, used by every ingress parse path — and
   `DispatchMetadata::internal(provenance, …)`, called only by the six
   internal dispatchers (all of which live in garyx-gateway). A
   `DispatchAuthority` capability parameter on `internal()` stays as a
   misuse tripwire (composition root creates it once), documented as
   hygiene, **not** as the security boundary. External ingress DTOs have
   no provenance field, so untrusted *data* has no path into
   `internal()` regardless of crate visibility.
2. **Controlled store writes (r5 BLOCKER 2).** The thread-history store's
   write surface is retyped:
   - `TrustedCommittedMessage` — produced only by the bridge persistence
     path from a live run's `DispatchMetadata`; the only type whose
     top-level `provenance` is written through.
   - `UntrustedImportedMessage` — the type for legacy boot import,
     provider-session import, raw append/rewrite/replace helpers, and
     any other `Value`-shaped entrance; its constructor **unconditionally
     strips top-level `provenance`** (and the legacy scattered keys).
     The current public raw-`Value` append/rewrite APIs become private or
     are re-exposed only as this untrusted import API.
   - The reducer reads `message.provenance` from committed records; that
     is safe *because* of this write-surface split, not because of the
     value's type.
3. **Migration sweep (§3.5)** deletes any pre-cutover top-level
   `provenance` JSON under the store lock; later import generations
   strip at the import boundary (no recurring global rescans).

`ProviderMessage` is not the trusted carrier: committed-record
construction goes through the typed store surface above, and
`ProviderRunOptions` loses its bulk serde derive in favor of the explicit
`PersistedRunOptions` projection (r4; unchanged).

#### Canonical record + history wire (r5 MAJOR 4)

- **Stored record**: top-level `provenance` object is the only stored
  truth for internal-dispatch semantics. The legacy top-level `internal`
  / `internal_kind` fields are **not stored** on new records; no behavior
  reads them anywhere.
- **History response**: `/api/threads/history` is not a raw pass-through
  today (it re-serializes via `ProviderMessage::from_value`), so the
  contract is stated honestly: the history message envelope **adds**
  `provenance` (additive field; consumers ignore unknown fields today)
  and keeps `internal`/`internal_kind` as **derived projection fields**
  computed from provenance at response time for display consumers
  (desktop's loop-continuation summary keeps working); nothing derives
  behavior from them. Golden fixtures: stored record (direct, queued,
  migrated) **and** full history response.
- Queue records (`PendingUserInput`) carry `provenance` +
  `durable_metadata` + identity fields incl. explicit `origin_run_id`
  (confirmed; both legacy filter constants deleted).

### 3.2 External ingress: field-level matrix from the actual surface

(r5 BLOCKER 3 — the evidence pass moved into the design; the matrix below
reflects the reviewer-verified current surface. Nothing shipped is
silently removed; nothing new becomes externally settable.)

| Surface | Typed, kept (today's shipped capability) | Handling |
|---|---|---|
| chat / `AtomicDispatchBody` | `model`, `model_reasoning_effort`, `model_service_tier`, `system_prompt` (request > thread > agent precedence per `agent_availability.rs`); `remote_mcp_servers` (**URL-only external schema**); `garyx_mcp_headers` (minus reserved) | run-scoped `RuntimeMetadata` via typed fields; never persisted |
| `CreateThreadBody` | existing typed product fields: `model`, reasoning, service tier, session resume, fork (Mac/iOS shipping) | stay typed thread fields; run-scoped runtime overrides rejected; agent snapshot moves to `ThreadRuntimeConfig` (internal writes only; bare-metadata `merge_thread_agent_runtime_snapshot` retired) |
| Telegram group/topic prompt | server-side channel configuration | **`ChannelRuntimeConfig`** — a trusted server-config channel, not external input, not extension data |
| built-in channels (`garyx-channels`) | prompt attachments, native command text, routing identity | dedicated typed content/context fields on `InboundRequest`; providers never reinterpret extension keys |
| plugin `deliver_inbound` | none | extension namespace only |
| all surfaces | generic/custom metadata | `metadata.external` extension namespace, inert |

Hard rules:

- **External MCP servers are URL-only.** The current schema accepts stdio
  `command/args/env/cwd`; that sub-shape becomes internal-only (gateway
  managed-MCP injection). External submissions of stdio shapes are
  rejected.
- **Reserved MCP headers cannot be spoofed.** `X-Run-Id`, `X-Thread-Id`,
  `X-Session-Key`, `Authorization` are written by the server **after**
  merging external headers (server wins), since the MCP end trusts
  headers over path.
- `provider_env`, `developer_instructions`, `desktop_antigravity_env`
  remain server-owned everywhere. (`system_prompt` is externally
  overridable **only** through the typed chat/atomic field above — the
  shipped precedence behavior — and still never persists.)
- No ingress DTO has a provenance field; forged provenance-shaped JSON in
  generic metadata lands inert in `metadata.external`.
- `runtime_context` rebuilt from authoritative typed fields; admission
  fingerprints over the typed form; thread-metadata copy-through carries
  typed/extension forms only.

Envelope restructure, `xml_attr` control-character encoding, and the
capability-block precondition are unchanged (confirmed).

### 3.3 Models: flattened presentation payload (confirmed; unchanged)

Reducer projects `presentation` from `message.provenance`
(TaskNotification / RestartWake, user-role rows); golden JSON both kinds;
discriminated unions both clients; full-payload signatures; rows-hash
upsert test; real Storybook row.

### 3.4 Clients (confirmed; unchanged)

Prose parsers deleted; structural decoder only behind server
presentation; shared user-role width owner; 10-line-box clamp with
injected ε and settling re-measure; expand = controlled Dialog /
always-attached cover with immutable snapshot selection.

### 3.5 History migration (skeleton confirmed; two additions)

Range-rewrite migration `structured_presentation_metadata_v1`, marker
identity `(migration_version, import_generation, thread_id)`:

1. **Secret scrub** (unchanged r5 list incl. `sdk_session_fork`; fork
   behavior re-homed onto typed `fork_from_*` + session binding).
2. **Provenance strip**: delete any pre-cutover top-level `provenance`
   JSON from committed records and pending inputs during the same locked
   pass (r5 BLOCKER 2); later import generations strip at the import
   boundary via `UntrustedImportedMessage`.
3. **Normalization without trust upgrade** (confirmed): legacy scattered
   keys removed; no legacy record is ever upgraded to provenance;
   historical task/restart messages keep plain-text rendering; forged
   fixtures use real task ids + plugin-controlled `task-notify-*` run ids
   **and** canonical top-level provenance JSON arriving via boot import,
   provider import, and raw append/rewrite — all must stay untrusted.

### 3.6 Wire cutover (confirmed, with the history clarification)

`render_schema = 2` on the render-state stream only; upgrade-required /
fail-fast both directions; client caches bump schema and hard-drop v1
snapshots; desktop+gateway atomic; iOS v2 minimum build. The history
endpoint's change is the additive envelope described in §3.1 — stated as
a wire addition, not "unchanged".

## 4. Non-goals

(unchanged) Bot-channel human rendering; task lifecycle semantics;
notification targets; delivery routing. Restart notices in scope;
retroactive certification rejected, not deferred.

## 5. Validation

- **Types/construction**: trybuild — `DispatchMetadata::internal` without
  authority fails; ingress DTOs deserialize with no provenance slot
  (compile-level absence); runtime container into persistence fails; bulk
  serialize of run options fails.
- **Store surface**: `UntrustedImportedMessage` strips top-level
  provenance across legacy boot import, provider-session import, raw
  append, rewrite/replace; `TrustedCommittedMessage` is the only
  provenance-writing path (negative: no public raw-`Value` write API
  remains).
- **Boundary**: sentinel-secret absence for every runtime key across all
  four dispatch paths; provenance retention for all six variants through
  the real busy-thread queue; golden stored-record trio + history
  response.
- **Ingress matrix**: per-surface tests — typed overrides reach the
  provider and never persist; stdio-shaped external MCP servers rejected;
  reserved-header spoof overwritten by server values; Telegram
  server-config prompt still reaches the provider via
  `ChannelRuntimeConfig`; CreateThread UI-visible typed fields
  (model/reasoning/tier/resume/fork) keep working from Mac/iOS clients;
  channel attachments/commands/routing arrive via typed fields; forged
  provenance/internal keys are inert (no lifecycle flip, no internal
  marking, no presentation); typed-form fingerprints (collision test).
- **Internal-field retirement**: no stored `internal`/`internal_kind` on
  new records; no behavior consumer remains (grep-level guard); history
  derives them for display only; desktop loop-continuation summary works
  from the derived fields.
- **Models/clients/prompt**: unchanged confirmed matrix.
- **Migration**: historical shapes + forged-history negatives (real task
  id, matching thread, legal envelope, plugin-chosen run id, canonical
  top-level provenance via every import path); secret-scrub assertions on
  jsonl + history API of the real-copy rehearsal; pre-cutover provenance
  strip; first-fork-after-scrub; three-point fault injection;
  per-generation marker dedup; old-cursor refetch.
- **Cutover**: v2×old, old×v2, additive history envelope golden,
  cache-schema upgrade + offline cold-open.
- **End-to-end evidence attribution** (unchanged r5): desktop worktree ↔
  installed `app.asar` hash equality before/after + renderer `--app-path`
  check; iOS worktree ↔ installed carrier (debug.dylib/executable) hash
  equality before/after; busy-thread E2E through committed record →
  collapsed card → dialog; iOS simulator flow; SwiftPM mapper tests.

## 6. Rollout

(confirmed; unchanged) One change set; desktop+gateway together;
migration on first boot; iOS normal release flow with v2 minimum build;
TestFlight only with explicit user approval in that turn.
