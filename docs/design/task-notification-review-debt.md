# Task Notification Review Debt (Slices B & C)

Status: debt register, owner decision 2026-07-21 — slice A shipped
separately (`task-notification-structured-presentation.md`); everything
below is recorded for future scheduling, **not** in flight.
Source: nine adversarial review rounds of #TASK-2541 (revisions 1–9,
`docs/design` git history `228846ce0..042fd35bd`). Each item cites the
round that established it. Findings were code-verified by the reviewer;
line references are as of 2026-07-21.

## Slice B — existing security debt (independent of the notification bug)

### B1. Provider runtime secrets persisted into transcripts (r1–r3, r8)

The direct persistence path clones the whole run metadata map minus a
two-key denylist (`persistence.rs` `RUNTIME_ONLY_METADATA_KEYS`), while
run-metadata backfill injects `provider_env` (may contain tokens),
`system_prompt`, `garyx_mcp_headers` (feeds MCP HTTP headers),
`desktop_antigravity_env` (arbitrary process env),
`developer_instructions`, `sdk_session_fork`. Slice A extends the
denylist so **new** records stop leaking; already-committed history
still contains these values and `/api/threads/history` serves them.
Full remediation = historical scrub (see C4) plus typed containers (C1).
Also: request-shaped fields (`model`, reasoning, tier,
`requested_provider_type`, workspace aliases) persist today and read as
caller input; the r8 three-type split (`ExternalRunOverrides` /
`ThreadRuntimeConfig` / `ResolvedRunAttribution` with `effective_*`
naming) is the clean shape.

### B2. Managed MCP identity is forgeable via loopback URL paths (r7–r9)

The Garyx MCP route derives run/thread identity from the URL path;
loopback requests bypass gateway auth (`gateway_auth.rs:165`); external
`remote_mcp_servers` entries currently accept stdio
`command/args/env/cwd` shapes and arbitrary URLs — an entry pointing at
`http://127.0.0.1:…/mcp/<victim-thread>/<forged-run>` inherits a forged
identity and can call mutation tools (schedule/capsule). Direction
agreed in review: a server-minted, run-bound `ManagedMcpCapability`
(CSPRNG, ephemeral registry `digest → {thread, run, generation}`,
mismatch = reject, revocation on terminal/abort/restart, never logged).
**Open problem (r9 B4)**: Claude Code drops custom headers/queries and
keeps only the URL path (`route_graph.rs:440`), so the capability needs
either a managed local proxy that injects headers, or path-carried
tokens with mandatory redaction — one mechanism, to be designed.
Related hardening: URL-only external MCP schema (stdio internal-only),
reserved-header filtering (`X-Garyx-Token`, `X-Mcp-Token`, `X-Run-Id`,
`X-Thread-Id`, `X-Session-Key`, `X-Channel`, `X-Account-Id`,
`Authorization` toward managed endpoints), ASCII case-insensitive,
applied to both `garyx_mcp_headers` and per-server `headers`, without
breaking third-party bearer auth.

### B3. External metadata forgery surface beyond chat (r3, r5, r6)

Slice A strips `task_notification` at chat + atomic dispatch only.
Remaining ingresses accept arbitrary metadata that flows toward
provider/persistence: `CreateThreadBody.metadata` (create-only keys are
later bulk-copied into dispatch metadata via thread-metadata
copy-through and `merge_thread_agent_runtime_snapshot` picks
runtime keys from bare thread metadata), plugin `deliver_inbound`
`extra_metadata` (arbitrary pass-through; owner already approved
deleting the wire field — note serde ignores unknown fields, so
deletion must be an explicit rejection, r9 m1), built-in channels
(`garyx-channels` construct `InboundRequest.extra_metadata` for
attachments/commands/routing — needs typed fields), admission
fingerprints computed over unsanitized maps. External callers can also
set `internal_dispatch`/`system`, which today gates task wake behavior
(`task_hooks.rs`) and internal record marking.

### B4. Restart notices still prose-parsed (r1, r4)

`<garyx_restarted>` is sniffed by two more client regex parsers with no
server presentation. Producer already has structured `restart_wake_*`
metadata (scattered keys). Folding it into the presentation mechanism
(a `restart_notice` kind projected from structured metadata) removes
the second dual mechanism. Blocked on nothing; small, independent.

## Slice C — architecture endgame (design sketches from r4–r9)

### C1. Typed dispatch metadata end to end

`DispatchMetadata { provenance, durable, runtime }` with
`external()`/`internal()` constructors (no provenance slot on the
external path), opaque non-serializable `RuntimeMetadata`, sealed
six-variant `ProvenanceRecord` (task notification, restart wake,
followup, automation, cron, task auto-start; serde tag `kind`, restart
field `wake_kind` to avoid the internal-tag conflict), per-row
provenance attribution replacing `build_run_messages` bulk merge,
`QueueCommitAttribution` as the sole source of queue keys.
Trust model: the write path is the root of trust — trusted vs untrusted
store write types (`TrustedCommittedMessage` / `UntrustedImportedMessage`
/ `UntrustedImportedThreadRecord` / pending equivalents + a dedicated
`apply_live_run_patch`), with open questions from r9 B1 (cross-restart
pending trust semantics; atomicity of the live-run patch across the
nine-field persistence write) recorded in the #TASK-2541 thread.

### C2. History envelope + internal-field retirement

Stored records carry provenance as the only truth; retire
`internal`/`internal_kind`/`loop_origin` across gateway, desktop
renderer, desktop main process (required-field decode today —
`threads.ts:145`), iOS, CLI task-progress annotation. History envelope
projects provenance at the outer level (single client read point).
Requires a version gate on history as well as the render-state stream
(`render_schema=2`, upgrade-required before payload), client persistent
cache schema bump (bodies + snapshots dropped together). r9 B3: goldens
must be generated by the real serializers, never hand-written
(`user_input` kind, `str/list` raw types, right-open index ranges,
`role_counts`/`include_tool_messages`/`kind_counts`).

### C3. External override typing

`ExternalRunOverrides` for chat (busy-thread conflict semantics needed:
reject non-empty overrides on `QueuedToActiveRun` at a side-effect-free
boundary — r9 M2); `ThreadRuntimeConfig` for Create/Update thread
(existing typed product fields; retire bare-metadata snapshot merge);
`ThreadRunBindingInput` for first workspace/provider binding (r9 B2 —
these persist to thread state by contract and are not one-shot);
`ChannelRuntimeConfig` for server-side channel prompts (Telegram
group/topic); `DurableRuntimeContext` as a fully typed nested schema
(bot/thread/task) rebuilt from authoritative state — consumers to
migrate: provider prompt env (`GARYX_TASK_ID/STATUS`), automation
memory (`memory_context.rs:217` reads `automation_id`), task hooks.

### C4. Historical data cleanup migration

A boot-time range-rewrite migration (protocol matured across r3–r9):
ordering import → migrate → serve; store lock; original seqs untouched;
typed-decoder field classification; scrub of runtime + request-shaped +
imported `effective_*` fields; pre-cutover provenance strip; **no
retroactive trust upgrade** (no provable ledger exists — task events die
with deletion, restart pending files are deleted on success, plugins
could forge anything; r5/r8 verdict); activity preservation must keep
exact `activity_seq`/timestamps via in-place UPDATE (allocation-free;
r9 B5 — `message_count` changes from appended markers must be either
declared expected deltas or control records excluded from canonical
counts, a system-level decision); per-generation markers; three-point
fault injection; rehearsal on a real-data copy asserting secrets absent
from jsonl and the history API.

## Review-method notes worth keeping

- The HEAD design document is the complete contract; "frozen in an
  earlier revision" is not a state (r6 lesson).
- Goldens must be produced by the real serializers and checked in
  (r8–r9 lesson: hand-written goldens drifted twice).
- Scope discipline: adjacent pre-existing defects discovered during
  review belong in a debt register (this file), not in the feature's
  blast radius — the nine-round expansion happened because every real
  finding was absorbed into one design (owner correction 2026-07-21).
