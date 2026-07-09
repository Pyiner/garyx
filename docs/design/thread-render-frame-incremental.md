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

Three knives, in dependency order. The end state has no full-`render_state`
live frames, no opt-in resume flag, and no dead fields — per the "do not
keep old logic" doctrine, with one explicitly bounded compatibility window
for iOS's independent release train.

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
    "from_seq": 2551,          // client must hold the snapshot at this seq
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
`last_render: Option<(u64 /*seq*/, HashMap<RowId, String /*serialized row*/>)>`.
After deriving the new snapshot, serialize each row once, diff by row id
against `last_render`, emit changed/new rows + the id order. First live
frame on a connection (no `last_render`, or after any replay frame) is a
full `render_state` frame that seeds the cache. Rationale for not sharing
across connections: desktop's hub multiplexes renderer windows into one
connection per thread, so real fan-out is 1–2 connections/thread — a
shared cache's complexity (locking, floor-keyed variants) buys nothing.

Client application (both ends, same semantics):

1. Validate `from_seq == local render snapshot's based_on_seq`. Mismatch ⇒
   discard the frame and enter the existing gap path (desktop: gap error →
   authoritative refetch; iOS: resume override reconnect). No new recovery
   machinery.
2. Rebuild rows in `row_order`: take the body from `upsert_rows` if present,
   else from the previous snapshot by id. A missing id (in neither) is a
   protocol violation ⇒ same gap path.
3. Replace the scalar fields wholesale; bump `based_on_seq`; run the existing
   dedupe guards unchanged (`based_on_seq` monotonic guard on desktop,
   `renderEquivalent`/flush gate on iOS).

Floor interaction: `row_order` is derived from the connection's own
windowed snapshot, so floors compose naturally; `window` rides along whole.

Compatibility: connections that do not send `render_mode=delta` receive
full frames. Desktop ships in lockstep with the gateway (same repo, same
release) and declares the mode immediately. iOS declares it from the next
TestFlight build; until the fleet moves, old iOS builds keep receiving full
frames. The full-live-frame path is deleted when the iOS floor version has
delta (tracked as the knife-4 cleanup gate) — the flag itself stays, as the
negotiation is what lets a curl/debug subscriber read frames at all.

Expected effect on the forensics thread: 358KB → ~10–15KB per commit frame
(~25–35×), and the per-commit CPU pulse shrinks by the serialization share.

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
  delta-apply(prev, delta) == render_snapshot_at_seq), (b) first-live-frame
  seeding, (c) mismatch ⇒ reconnect error, (d) default windowed degrade
  fires without the flag, (e) deleted field absent.
- Sabotage red-green: break the diff (drop one changed row) and assert the
  oracle test fails; break from_seq validation and assert the gap test fails.
- Desktop: mirror contract tests feed delta frames through `applyFrame`;
  electron-smoke mock updated to the new contract. One packaged-app check
  (dist:dir) since main-process stream code changes.
- iOS: GaryxMobileCore SwiftPM tests for decode + apply + gap fallback,
  driven by captured frame fixtures; no pbxproj changes expected (Core-only
  for the state machinery).
- On-device: repeat the 8-subscriber forensics; expect commit-pulse CPU and
  per-subscriber byte accounting to collapse (12.42MB/90s → ~1MB dominated
  by the seed frame).

## Batches (each lands with its own codex review)

1. models+gateway: `render_mode=delta`, per-connection diff, delta frames,
   oracle tests. (Full frames still default.)
2. desktop: declare delta, apply path, contract tests, packaged check.
3. iOS: declare delta, Core apply path, SwiftPM tests.
4. cleanup: flip windowed-resume default + delete the parameter; delete
   `visible_message_ids`; delete the full-live-frame path once the iOS floor
   carries batch 3 (this last deletion may trail the rest).
