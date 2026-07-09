# Thread render frame incrementalization (#TASK-1956 B-line)

Status: draft, pending review.

## Problem, quantified

Per-thread SSE (`/api/threads/{id}/stream`) resends the **entire**
`render_state` in every live frame. Forensics on a real 2,539-record thread
(office gateway, 2026-07-09):

- Live frame per commit: **~358KB** — `rows` 107 rows / 331KB (whole array
  every time) + `visibleMessageIds` 26KB (dead weight, see below). During an
  active agent run this is re-derived, re-serialized, and re-sent per commit,
  per connection. Measured CPU pulses of 155–230% with 8 subscribers,
  10–46% in the daily 1-subscriber shape.
- Bare-cursor first frame: **8.96MB**, of which `events` replay is 9.2MB
  (2,676 committed records) and `render_state` only 358KB. Production
  clients are protected today only because both desktop and iOS opt in to
  `windowed_resume=1`; any consumer that forgets the flag re-inherits the
  full-replay cliff.
- Control-plane latency is NOT harmed anymore (p99 14.6ms under an 8×
  subscriber + commit storm — the batch-3 read-pool/spawn_blocking isolation
  holds). This is a CPU/bandwidth debt, not a latency emergency: the frame
  contract is the last all-quantity path left in the stream stack.

## Current architecture facts (verified 2026-07-09)

Server (garyx-gateway/src/routes.rs):

- Live path: one global broadcast bus; each SSE connection filters by
  thread_id and **independently** derives + serializes a full
  `RenderSnapshot` per forwarded commit (`committed_thread_stream_live_event`
  → `thread_render_snapshot_at_seq`). SSE `id` = seq; derive-mismatch ⇒
  reconnect error.
- Replay path: `build_thread_stream_replay` — `replay_scope=initial` +
  `initial_user_turns` (cold-open window), `windowed_resume=1` (opt-in
  degrade when serialized replay > 1MiB → 3-user-turn window,
  `replay:"windowed"` marker), gap self-heal via forward paging,
  `THREAD_TRANSCRIPT_REPLAY_CAP=10_000` backstop. Caught-up connect emits a
  snapshot-only frame (`events: []`).
- `render_floor > 0` ⇒ `render_snapshot_in_window` (rows from
  `[floor..tail]`, run_state from the full prefix).

Clients:

- Both ends dumb-render `render_state.rows` (server owns structure), resolve
  row → message bodies from their local committed cache, and append local
  optimistic user rows. Neither end patches rows incrementally today.
- Desktop: main-process hub holds **one connection per thread** (renderer
  windows share it); cursor = `after_seq` query + `Last-Event-ID` header;
  pins `render_floor` monotonically from `window.floor_seq`; sends
  `windowed_resume=1`; dedupes re-renders via the `based_on_seq` monotonic
  guard. Gap ⇒ error event ⇒ authoritative refetch.
- iOS: cursor = `after_seq` query only (no Last-Event-ID); sends
  `windowed_resume=1`; `replay_scope=initial&initial_user_turns=3` when it
  has no windowed snapshot; `renderEquivalent` + 3s leading-edge flush gate
  dedupe re-renders. Gap ⇒ resume override; 4 consecutive failures ⇒
  fallback polling.
- Only desktop + iOS consume this endpoint in production (no CLI consumer).
- `visibleMessageIds` has **zero consumers**: rust emits it, desktop only
  declares the type, iOS only decodes/encodes it. 26KB/frame of dead weight.

## Design

Three knives, in dependency order. The end state: both production clients
speak delta, the opt-in resume flag and the dead field are gone. Full
frames remain the negotiated default for undeclared consumers (old iOS
builds, curl, debug tooling) — that is a permanent, zero-cost contract
surface, not a compatibility layer.

### Knife 1 — delta live frames (`render_delta`)

Contract. A connection may declare `render_mode=delta`. On such a
connection, live frames (and only live frames — replay/snapshot-only frames
stay full) carry `render_delta` instead of `render_state`:

```jsonc
{
  "type": "thread_render_frame",
  "thread_id": "…",
  "events": [ /* unchanged: committed_message records, cursor/gap source */ ],
  "render_delta": {
    "from_seq": 2551,          // client must hold the snapshot at this seq…
    "from_rows_hash": "…",     // …with exactly this rows content (drift tripwire)
    "based_on_seq": 2552,
    "row_order": ["row-id", …], // full id sequence (~3KB @ 107 rows): re-order is unambiguous
    "upsert_rows": [ /* full RenderRow bodies, only new/changed rows */ ],
    "tailActivity": "…",        // small scalar fields always sent whole
    "activeToolGroupId": null,
    "progress_locus": "…",
    "rateLimit": null,
    "window": { "floor_seq": 0, "has_more_above": false },
    "filtered_placeholders": []
  }
}
```

Server derivation. Per-connection (not shared): the live loop already owns
per-connection state (`sent_committed_payloads`, `last_sent_seq`); add
`last_render: Option<(u64 /*seq*/, u64 /*rows hash*/, HashMap<RowId, u64 /*row hash*/>)>`.
`RenderRow` derives `Hash` (pure data); after deriving the new snapshot,
hash each row structurally, diff hashes by row id against `last_render`,
and **serialize only the changed rows** — the diff itself costs no
serialization. `rows_hash` (the chain token, below) is the hash of the
per-row hashes in `row_order` — also serialization-free.

Seeding rule (one rule, everywhere): **every frame that carries a full
`render_state` — replay, snapshot-only, or a full live frame (first on the
connection, or a same-seq reseed) — immediately sets the delta base on
both server and client to that frame's snapshot; the very next live frame
may be a delta.** There is no other cache-invalidation event: floor
advances arrive as replay frames and are covered by this rule.

Rationale for not sharing across connections: desktop's hub multiplexes
renderer windows into one connection per thread, so real fan-out is 1–2
connections/thread — a shared cache's complexity (locking, floor-keyed
variants) buys nothing.

Same-seq overwrites (finding 1). The gateway deliberately forwards a
changed payload at an already-sent seq (rewrite paths;
`should_forward_committed_payload` dedupes only identical payloads), and
desktop's monotonic render guard would silently drop a same-seq render
update — so a delta base could drift without tripping a plain seq check.
Two defenses, both mandatory:

- Server: when a live commit re-lands on `seq == last_sent_seq` (payload
  changed), do **not** emit a delta. Emit a full `render_state` frame and
  reseed the diff cache. Rewrite flows already end in an authoritative
  refetch on both clients (`refetch_authoritative` / control-rewrite
  planner), so the transient full frame is belt-and-suspenders, not the
  primary recovery.
- Client: `from_rows_hash` must equal the rows-hash the client holds. Any
  base drift — same-seq drops, guard interactions, future bugs — becomes an
  explicit gap-path exit instead of silent mis-render.

Hash contract (hash chaining — the server is the only hasher). Clients
never compute a hash: cross-language canonical JSON (Rust `None` ⇒ `null`
vs Swift `encodeIfPresent` omission, number formatting, key order) is
exactly the kind of contract that rots. Instead:

- Every frame that carries a full `render_state` also carries
  `render_state.rows_hash`: the server's combined hash over its per-row
  structural hashes in row order (algorithm is a server implementation
  detail; clients treat it as an opaque token).
- Every delta frame carries `from_rows_hash` and `rows_hash` (the value
  after applying this delta).
- The client transport layer stores the last accepted token and compares
  `from_rows_hash` by equality; on accept it stores the frame's
  `rows_hash`. Chain intact ⇒ the client's reassembled rows are the
  server's rows (the server only advances its own chain over states it
  actually emitted); chain broken ⇒ gap path.

The chain lives in the transport layer (hub forwarder / stream actor),
which sees every frame — renderer-side dedupe guards can never desync it.

Client application (both ends, same semantics):

1. Validate `from_seq == local snapshot.based_on_seq` **and**
   `from_rows_hash == stored rows-hash token`. Mismatch ⇒ discard the frame
   and enter the existing gap path. No new recovery machinery.
2. Rebuild rows in `row_order`: take the body from `upsert_rows` if present,
   else from the previous snapshot by id. A missing id (in neither) is a
   protocol violation ⇒ same gap path.
3. Replace the scalar fields wholesale; bump `based_on_seq`.

Where the client reassembles (finding 2). Delta reassembly and validation
live in each platform's **transport layer**, not the render layer:

- Desktop: in the main-process stream parser (`mapThreadRenderFrameEvent` in
  gary-client/stream.ts), which already owns `ThreadStreamGapError` — a
  failed validation throws it and rides the existing hub stop → gap error →
  authoritative refetch pipeline, socket teardown included. The reassembled
  **full** `render_state` is what gets emitted to the renderer:
  `DesktopChatStreamEvent`, `GatewayMirror.applyFrame`, the frontier guard,
  and every renderer contract stay byte-identical to today. The per-thread
  previous-snapshot cache lives next to the forwarder state in the hub.
- iOS: in `GatewayStreamFrameProcessor` (GatewayStreamActor), which already
  owns the `.gap(resumeAfterSeq:)` exit; the emitted action stream still
  carries a full snapshot, so `applyThreadRenderSnapshot`, the mapper, and
  `renderEquivalent`/flush-gate dedupe are untouched.

This confines knives' client work to two transport files + tests; the
render layers never learn deltas exist.

Ordering with local optimistic rows: deltas apply to the authoritative
server snapshot **before** local pending user rows are overlaid — which is
the existing order on both ends (server rows first, optimistic append
after); stated here as an explicit invariant.

Floor interaction: `row_order` is derived from the connection's own
windowed snapshot, so floors compose naturally; `window` rides along whole.
A floor change mid-connection arrives as a replay frame, which seeds a new
base per the seeding rule above — the diff cache needs no separate
floor-keying.

Compatibility: `render_mode=delta` is a permanent negotiation, not a
transition flag. Connections that do not declare it — old iOS builds, curl,
debug tooling, tests — receive full frames indefinitely; the full live
frame is the same constructor the replay/snapshot-only paths use, so
keeping it costs nothing. Desktop ships in lockstep with the gateway and
declares the mode immediately; iOS declares it from the next TestFlight
build. (Supersedes the earlier "delete the full-live-frame path" cleanup
item, which contradicted debug readability — resolved in favor of
default-full.)

Expected effect on the forensics thread: **wire** per commit frame 358KB →
~10–15KB (~25–35×), with the same reduction in downstream IPC/JSON.parse
and re-render diffing on both clients. Server-side snapshot **derivation**
cost is explicitly unchanged (File-store cached-tail already bounds it);
row serialization drops to changed-rows-only. The on-device validation
below measures bytes and CPU separately so the claim stays honest.

### Knife 2 — windowed resume becomes the default

`windowed_resume=1` is a transitional opt-in; both production clients
already send it. Flip the default: any resume whose verbatim replay would
exceed `THREAD_STREAM_RESUME_REPLAY_BYTE_BUDGET` (1MiB) degrades to the
cold-open user-turn window with the `replay:"windowed"` marker —
unconditionally. Delete the `windowed_resume` query parameter and its
plumbing (`ThreadStreamParams.windowed_resume`, options flag, both branch
arms). Clients keep sending the parameter harmlessly until their next
release removes it (unknown query params are ignored).

The gap self-heal forward-paging path stays for sub-budget gaps; the 10k
record cap stays as the absolute backstop. Smoke/e2e scripts that relied on
verbatim full replay assert on the windowed shape instead.

This retires the 8.96MB bare-cursor cliff for every consumer, present and
future, instead of guarding only flag-carrying ones.

### Knife 3 — delete `visible_message_ids`

Zero consumers on all three ends (verified above). Remove the field from
`RenderSnapshot` (garyx-models), the desktop contract type, the iOS
decode/encode, and fixtures. −26KB per full frame, −O(messages) per derive.
Pure deletion, no behavior change.

## What this deliberately does not do

- No event-stream changes: `events` remain the body/cursor/gap source of
  truth; committed replay semantics, seq assignment, and the broadcast bus
  are untouched.
- No shared cross-connection render cache (fan-out is 1–2 per thread; the
  per-connection diff cache is strictly simpler and lock-free).
- No client-side row derivation: rows stay server-owned
  (repository-contracts.md transcript-rendering rules unchanged; the delta
  is a transport encoding of the same server-derived rows).
- No changes to cold-open (`replay_scope=initial`) or `render_floor`
  pagination — knife 1 composes with both.

## Validation plan (headless-first)

- Gateway: routes tests drive a captured real-thread record stream through
  a delta-mode connection; assert (a) frame-by-frame reassembly on the
  client algorithm equals the full snapshot at every seq (structural oracle:
  delta-apply(prev, delta) == render_snapshot_at_seq, and the rows_hash
  token chain is preserved across full and delta frames), (b)
  first-live-frame seeding, (c) seq or hash mismatch ⇒ reconnect error, (d)
  default windowed degrade fires without the flag, (e) deleted field absent.
- Adversarial oracle cases (finding 1 family): same-seq overwrite mid-run ⇒
  full-frame reseed, then delta resumes cleanly; range-rewrite /
  transcript-reset control records interleaved with deltas; snapshot-only
  replay frame interleaved between deltas ⇒ reseed; floor advance
  (windowed replay) mid-connection ⇒ reseed.
- Sabotage red-green: break the diff (drop one changed row) ⇒ oracle test
  fails; break from_seq validation ⇒ gap test fails; break the hash ⇒
  drift-tripwire test fails.
- Desktop: unit tests target the main-process reassembler in
  gary-client/stream.ts (delta → full render_state, gap throw on
  seq/hash/row-id violations); mirror/renderer contract tests are
  unchanged by construction — one guard test asserts the renderer-facing
  event still carries a full snapshot. electron-smoke mock updated only in
  its stream fixtures. One packaged-app check (dist:dir) since
  main-process stream code changes.
- iOS: GaryxMobileCore SwiftPM tests for the frame-processor reassembly +
  gap fallback, driven by captured frame fixtures; mapper tests unchanged;
  no pbxproj changes expected (Core-only).
- On-device: repeat the 8-subscriber forensics, reporting wire bytes and
  CPU separately; expect per-subscriber bytes 12.42MB/90s → ~1MB (dominated
  by the seed frame) and the commit-pulse CPU to drop by the
  serialization + push share (derivation share explicitly remains).

## Batches (each lands with its own codex review)

1. models+gateway: `render_mode=delta`, per-connection hash diff, delta
   frames, same-seq full-frame reseed, oracle + adversarial tests. (Full
   frames remain the default for undeclared connections — permanently.)
2. desktop: declare delta, main-process reassembler + tests, packaged check.
3. iOS: declare delta, frame-processor reassembly + SwiftPM tests.
4. cleanup: flip windowed-resume default + delete the parameter; delete
   `visible_message_ids` on all three ends.
