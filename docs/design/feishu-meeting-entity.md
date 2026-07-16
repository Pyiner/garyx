# Feishu meeting entity

Status: revision 4, addressing adversarial review #TASK-2337 rounds 1–3
Author: gary (design)
Supersedes: `docs/design/feishu-group-listen-mode.md` (group listening product
surface is cancelled). Meeting **content** never enters thread ledgers; Phase
1 ships zero background ledger writes (Section 7.3).

## 1. Summary

When someone invites the Garyx bot into an ongoing Feishu meeting, the bot
joins, polls in-meeting events, and materializes a first-class **meeting
entity**. Capture is durably checkpointed at bounded tick granularity; a
crash never loses closed content and never loses more than the current
bounded tick. When the meeting ends, capture drains the platform's grace
window and the entity becomes immutable.

Agents read entities only through `garyx meeting read`. The server keeps a
per-(entity, thread) integer segment cursor with a fetch/confirm two-phase
protocol executed inside the single CLI invocation: delivery is
at-least-once for unconfirmed spans, silent skips are impossible, and
oversized content is split into independent segments **at write time** so
the cursor's unit is always a whole segment.

All user-turn entry paths — fresh runs, queued follow-ups, and the direct
`/api/chat/stream-input` route used by desktop and iOS — share one
`prepare_user_turn` seam in the bridge, which injects the meeting context
block into provider input and stamps server-owned refs into acknowledged
user metadata.

## 2. Product contract (user-approved, 2026-07-16)

1. Meetings only.
2. Start: an in-meeting invite admits, joins, and starts polling. Manual
   `garyx meeting join` is a debug path.
3. Live: near-real-time accumulation (~30 s transcript latency is a
   platform property).
4. Read protocol: CLI-only access; per-thread server cursors; incremental
   by default; self-describing output.
5. End: the meeting-ended signal stops capture and freezes the entity.
6. Reference continuity: recognition of prior references is server state.

Product sign-off items (accepted limits):

- Feishu WS ACKs before processing. An invite is lost iff the process dies
  or the admission insert exhausts bounded retries (3 attempts with
  backoff, error-logged) in the ACK→admission window.
- One entity per admission: re-inviting after terminal/deletion creates a
  new entity capturing from that point.
- A meeting's final ~30 s of speech becomes readable during the grace
  drain.
- A crash can lose at most the current in-memory tick, which is bounded
  (Section 4.2); such loss is detectable (checkpoint chain gap vs platform
  re-pull) and healed by the idempotent re-pull.

## 3. Verified repository baseline

B1–B12 as confirmed by review rounds 2–3:

| # | Fact | Evidence |
|---|---|---|
| B1 | WS ACKs before processing; only `im.message.receive_v1` handled; 30 min event-id dedup | `ws.rs:968,1040,1049-1072`, `feishu.rs:95` |
| B2 | `FeishuChannel::new(...)` → `FeishuRuntimeContext` injection | `feishu.rs:370`, `ws.rs:38-69` |
| B3 | gateway depends on channels; channels-defined/gateway-implemented trait compiles; this is the first such trait | `garyx-gateway/Cargo.toml:20`, `plugin.rs:3235,3322` |
| B4 | `FeishuClient` is `Clone` but `pub(crate)` | `client.rs:127` |
| B5 | `CronService` background-service shape | `cron.rs:633,713-796,799-851` |
| B6 | Capsule storage precedent | `capsules.rs:158,273`, `garyx_db/mod.rs:2645,3559` |
| B7 | No safe background control-record seam; foreign tail rows trigger reconcile re-append | `persistence.rs:364`, `store.rs:521` |
| B8 | `GARYX_THREAD_ID` env injection | `gary_prompt.rs:148-154` |
| B9 | CLI mutation timeout 10 s | `gateway_client.rs:15` |
| B10 | `gary_prompt` is a synchronous formatter | `gary_prompt.rs:36,73,125` |
| B11 | No global client event route; no meeting SSE in Phase 1 | `event_stream_hub.rs:6-49`, `routes.rs:2369` |
| B12 | Thread delete/archive are their own transaction paths | `garyx_db/mod.rs:650,2036` |
| B13 (corrected, round 3) | `start_agent_run` is NOT a universal seam: `/api/chat/stream-input` calls `add_streaming_input` directly (used by desktop and iOS), and the queued branch forwards only a five-field attribution allowlist to the provider | `application/chat/control.rs:30`, `desktop .../stream.ts:944`, `GaryxMobileModel+Composer.swift:298`, `run_management.rs:94,254` |

External platform facts: as prior revisions. `page_token` is opaque —
stored and resumed from, never compared. Error typing (RR3-05):
`MeetingApiError = NotInMeeting (10005) | GraceExpired (20001) |
RetriableTransport | Other(code, msg)`. `20001` means the post-end window
is already over. Whether an in-band "meeting ended" page item exists is
**pinned by the sample gate** (3.1); if the fixtures show none, the only
end sources are the push event and `GraceExpired`.

### 3.1 Sample-pinning gate

Slice 2 opens by capturing sanitized fixtures: invite/ended envelopes, join
response, events pages (including, if reproducible, pages read during the
grace window and the `20001` response). Pinned facts: invite payload
fields; joining-stage identity; existence or absence of an in-band ended
item; grace-window read behavior. Mismatch with this design stops slice 2
for an amendment. Slice 1 has no Feishu dependency.

## 4. Entity model and storage

### 4.1 SQLite

`meetings` (STRICT; all timestamps UTC RFC3339 `Z`):

```
id / account_id / meeting_no / feishu_meeting_id ('' until join)
invite_event_id / call_id / topic / invited_by
status CHECK(status IN ('joining','live','finalizing','aborting','finalized','aborted'))
status_detail TEXT NOT NULL DEFAULT ''
stalled_reason TEXT NOT NULL DEFAULT ''  -- '' | 'no_client' | 'auth_failed' | 'transport'
end_source TEXT NOT NULL DEFAULT ''      -- '' | 'push' | 'poll_ended'(if pinned) | 'grace_expired'
join_deadline_at / grace_deadline_at
poll_cursor (cache; log checkpoint chain is truth)
closed_segment_count / byte_size (caches)
started_at / ended_at / finalized_at / created_at / updated_at
```

Uniqueness: `UNIQUE(invite_event_id)`; partial unique
`(account_id, meeting_no)` and `(account_id, feishu_meeting_id) WHERE
feishu_meeting_id <> ''`, both over
`status IN ('joining','live','finalizing','aborting')` — one **active**
entity per meeting per account; terminal/deleted never blocks a fresh
admission (§2).

`meeting_read_cursors` (STRICT):

```
meeting_id REFERENCES meetings(id) ON DELETE CASCADE
thread_id
confirmed_seq INTEGER NOT NULL DEFAULT 0
pending_from / pending_to INTEGER
receipt TEXT
updated_at
PRIMARY KEY (meeting_id, thread_id)
CHECK ((pending_from IS NULL) = (pending_to IS NULL)
   AND (pending_from IS NULL) = (receipt IS NULL)
   AND (pending_from IS NULL OR
        (pending_from > confirmed_seq AND pending_to >= pending_from)))
```

Cursors are whole-segment integers only; there is no sub-segment position
anywhere in the protocol (RR3-02 resolved at write time, 4.2).

`meeting_thread_refs` (STRICT) — a **canonical relation** (the attach API
is its single writer; it is the source of truth for "which threads hold a
ref", satisfying the no-body-scan rule):

```
meeting_id REFERENCES meetings(id) ON DELETE CASCADE
thread_id / attached_at
PRIMARY KEY (meeting_id, thread_id)
```

Indexes: `idx_meetings_updated(updated_at DESC, id)` (keyset pagination,
RR3-09), `idx_meetings_status(status)`,
`idx_refs_thread(thread_id, attached_at DESC)`.

Thread delete/archive transactions also delete this thread's cursor and
ref rows; `/read` and `/refs` validate thread existence (and non-archived
state, for attach) inside their transactions (RR3-08).

### 4.2 Content: bounded-tick checkpointed segment log

`~/.garyx/meetings/{entity_id}/segments.jsonl`, two line kinds:

```
{"t":"seg","seq":12,"kind":"transcript","speaker":"张三","start":"…","end":"…",
 "text":"…","sources":["sent_8813","sent_8814"]}
{"t":"ckpt","cursor_out":"pt_x9y8…","at":"…"}
```

**Write-time size splitting (RR3-02):** before a segment is appended, text
larger than 32 KiB (half the default response budget) is split at UTF-8
boundaries into consecutive independent segments, each with its own seq
(`"cont":true` marks continuations for rendering). The read protocol,
cursors, and receipts therefore only ever address whole integer segments.

**Bounded tick protocol (RR3-04):** a tick pages from the persisted
cursor. The tick **closes** — mandatorily — when any bound is hit:
`has_more=false`, 10 pages, 1000 items, 1 MiB accumulated text, or 10 s
wall time. Closing a tick is the single durability sequence, under the
entity I/O lock:

1. close all open coalescing (coalescing never crosses a tick close);
2. append `seg` lines → append one `ckpt` line (every tick closes with a
   ckpt, including empty/all-duplicate ticks) → `fdatasync`;
3. update SQLite caches in one transaction.

If bounds forced the close with `has_more=true`, the coordinator
immediately schedules the next tick (continuous small ticks under high
volume — content becomes durable and readable incrementally). Terminal
signals and all queue commands are serviced **only at tick closes**
(RR3-04/RR3-05 arbitration point); a signal arriving mid-tick waits for
the bounded close (≤ 10 s).

Dedup on replay/re-pull is by `sources` ids. **Boot repair** (only for
non-terminal entities or logs marked dirty): validate line by line,
truncate a torn tail, take the last valid `ckpt` as the resume cursor
(SQLite cache corrected from it), keep `seg` lines after it (re-pull is
idempotent), rebuild counters, and build the in-memory offset index in the
same pass (RR3-09). Terminal logs are not scanned at boot.

Markdown is a render format; no header/end-marker segments exist.

### 4.3 Entity deletion

Only terminal entities are deletable. Under the entity I/O lock:
`rename → tombstone`, DB delete (cascades), remove tombstone. **A missing
content dir is a legal empty entity** (a joining-deadline abort may never
have appended anything): delete then skips the rename and runs only the DB
protocol (RR3-07). Boot order: **reconcile tombstones first, then repair
logs** (RR3-07) — tombstone+row → rename back; tombstone without row →
remove; bare orphan dir without row → logged, removed.

API: `DELETE /api/meetings/{id}` (409 non-terminal) + `garyx meeting
delete`.

## 5. Ingestion

### 5.1 Platform client seam and registry

As revision 3 (`MeetingPlatformClient` + `MeetingEventSink` with
`register_client`/`unregister_client`, adapter inside garyx-channels,
non-blocking admission insert with 3 bounded retries, typed errors — now
including `GraceExpired`).

**Stalled semantics, per state (RR3-06):**

- `stalled_reason` is persisted and shown by `garyx meeting list`
  (`no_client` — registry has no client for the account; `auth_failed` —
  client present but calls fail with auth-class `Other`; `transport` —
  continuous transport failure > 15 min).
- **joining**: the absolute `join_deadline_at` always applies — it models
  invite validity, and a meeting will not wait for our client registry.
  Deadline expiry aborts even while stalled (single rule, no pause).
- **live**: never auto-aborted for stalled reasons (content is safe;
  capture may resume). `NotInMeeting` remains the only automatic abort.
- **finalizing**: abort is refused entirely. If the drain cannot run
  (stalled), the entity still transitions to `finalized` at
  `grace_deadline_at` — the drain window has passed either way, and the
  endgame of finalizing is always `finalized` (this also removes the
  "admin abort of stalled-finalizing" contradiction).
- Admin `garyx meeting abort <id>` / `POST /api/meetings/{id}/abort`
  applies to `joining` and `live` only.

### 5.2 Coordinator, admission, joining

As revision 3 (per-entity coordinator; durable CAS; intent stages
`aborting`/`finalizing`; admission insert = the only sink work; lazy
`ensure_dir`; join retry every 20 s to the absolute deadline), with one
arbitration refinement (RR3-05): terminal-affecting commands
(`EndedSignal`, `AbortRequest`, `NotInMeeting`) are collected and resolved
at tick closes with deterministic priority **end > abort**; if both are
queued, the end path wins and the abort is dropped as subsumed.

Join succeeding during the grace window is legal: subsequent polls return
events until the window closes with `GraceExpired`, which finalizes (the
"end during joining" case needs no cross-identity correlation; corrected
per RR3-05 — polls during grace do return data, the end comes from
`GraceExpired`, or earlier from a pinned in-band ended item if fixtures
show one).

### 5.3 End path and grace drain

- `EndedSignal(push)` (or `poll_ended` if pinned): CAS `live→finalizing`,
  `ended_at=now`, `grace_deadline_at = now + 4 min`, drain on normal
  cadence **to the deadline** (no quiescence shortcut), then CAS
  `finalized`.
- `GraceExpired` while live: window already over — CAS `live→finalizing`
  with `end_source='grace_expired'` and complete immediately (final ckpt,
  CAS `finalized`).
- `GraceExpired` while **finalizing**: accelerates completion — the drain
  stops now and finalization completes (not a dropped CAS loser, RR3-05).
- `NotInMeeting` while finalizing: complete early (no more data readable).
- Unknown-entity end signals: logged, dropped (the grace-window poll
  behavior above is the correlation-free backstop).

### 5.4 Lifecycle state machine

```
 invite ─admission insert─> JOINING ─join ok─> LIVE ─bounded ticks─┐
   │        (unique keys)      │(deadline,       │                  │ (commands
   │                           │ incl. stalled)  │ EndedSignal/     │  resolved at
   │                           v                 │ GraceExpired     │  tick closes;
   │                       ABORTING ─flush─> ABORTED                │  end > abort)
   │                           ^                 │                  │
   │                           └──10005 live─────┤                  │
   │                                             v                  │
   │                                        FINALIZING ─drain to────┘
   │                                             │  deadline; GraceExpired/
   │                                             │  10005 ⇒ complete early;
   │                                             │  abort refused; stalled ⇒
   │                                             v  finalize at deadline
   │                                        FINALIZED
 boot: tombstone reconcile → log repair (non-terminal only, builds offset
 index) → coordinators resume persisted stages.
```

## 6. Read protocol

### 6.1 CLI surface

- `garyx meeting list [--json]` — keyset-paged (`(updated_at, id)` cursor
  with a snapshot boundary token; stable under live-row updates, RR3-09).
- `garyx meeting read <entity_id> [--full] [--range A..B] [--thread <id>] [--json]`
  — incremental (default; requires thread identity via `GARYX_THREAD_ID`
  or `--thread`) or stateless paged snapshot (`--full`, `--range`; no
  identity, no cursor interaction). `--again` is removed (RR3-03): the
  header always names the exact span just delivered, so re-reading any
  span is `--range` — one mechanism instead of two.
- `garyx meeting join <no>` / `abort <id>` / `delete <id>`.

### 6.2 Gateway API, locking, index

Routes as revision 3 plus `snapshot`/continuation parameters on `/read`.

- **Entity I/O lock**: per-entity `RwLock` covering all states including
  terminal (read vs append vs finalize-flush vs delete-rename all
  serialize here; terminal entities take it directly, live via
  coordinator).
- **Offset index (RR3-09)**: sparse (every 64th segment → byte offset).
  Live entities: built during boot repair and maintained per append.
  Terminal entities: persisted as a derived, rebuildable
  `{id}/index.bin` written at finalize; if missing on first read it is
  rebuilt once and re-persisted. Boot never scans terminal logs; cold
  high-seq reads are O(span) via the persisted index.
- **Stateless snapshot continuation (RR3-03)**: the first `--full`/
  `--range` response fixes a snapshot `(closed_latest, log_offset)` and
  returns an opaque continuation token binding
  `{entity_id, snapshot, next_seq, mode, range_end, checksum, expiry 10 min}`.
  The CLI loops within one invocation, streaming pages to stdout until the
  snapshot is exhausted (or `--max-bytes` stops it early, printing the
  resume command with the token). Live appends beyond the snapshot are
  invisible to that snapshot's pages.

### 6.3 Fetch/confirm two-phase (incremental)

As revision 3: fetch never advances `confirmed_seq`; it re-serves an
existing pending span or creates one (up to the page cap) with a fresh
opaque receipt. Confirm CAS-matches
`(confirmed_seq, pending_from, pending_to, receipt)`, advances, clears.
The CLI runs fetch → render → stdout flush → confirm inside one
invocation.

Corrected failure statements (RR3-03):

- Crash/broken pipe **before** stdout flush → no confirm sent → pending
  survives → the same span re-serves next time (at-least-once).
- Confirm **committed but its response lost** → pending is already
  cleared; the next read serves the *next* span. This is safe — the
  content was fully flushed to stdout before confirm was attempted — and
  is distinct from the previous case in the test matrix.
- Cursors never regress; row CAS serializes concurrent readers.

### 6.4 Self-describing output

As revision 3, minus `--again` (the header's "re-read this span:
`--range 12..18`" line replaces it). Every response names: mode, exact
span, totals, live/terminal status + `end_source`/`stalled_reason` when
set, and the follow-up commands.

## 7. Pointer injection and references

### 7.1 One seam: `prepare_user_turn` in the bridge (RR3-01)

B13 correction makes the seam explicit. `garyx-bridge` gains one internal
choke point through which **every** provider-bound user turn passes:

```rust
async fn prepare_user_turn(&self, thread_id, text, metadata) -> PreparedUserTurn
// PreparedUserTurn { provider_input, acknowledged_metadata }
```

Callers: `start_agent_run` (fresh turns) **and** `add_streaming_input`
(both its `start_agent_run`-internal queued branch and the direct
`/api/chat/stream-input` route used by desktop/iOS — the route's bridge
entry is `add_streaming_input`, so instrumenting the bridge method covers
HTTP/WS, desktop, and iOS without touching clients).

`prepare_user_turn`:

1. invokes the `MeetingContextResolver` (bridge-defined trait,
   gateway-implemented, assembly-injected);
2. builds `provider_input` = context block + original text — the
   transcript-persisted text stays exactly what the user wrote;
3. stamps server-owned `meeting_ref` entries into the metadata that will
   be acknowledged/committed for the user message — the queued-input
   attribution allowlist (`run_management.rs:94`) is extended with this
   server-owned field so queued turns carry it too (RR3-01);
4. discards any caller-supplied `meeting_context`/`meeting_ref` metadata
   (spoof-proof: `/api/chat/stream-input` has no metadata surface and
   ChatRequest metadata is untrusted — refs come only from the canonical
   relation).

Resolver failure semantics (RR3-08): if the initial
`meeting_thread_refs` point-read fails, the turn fails **retryably**
(surfaced like other transient dispatch errors) — recognition of existing
refs is a product contract and silently proceeding would break it. If refs
load but entity/cursor point-reads fail, a degraded stub block with entity
ids and `context: unavailable (retryable)` is injected. Topic and all
platform text are encoded as deterministic JSON strings (quotes CR/LF and
control characters — no line-injection surface) and length-capped
(RR3-08).

### 7.2 Ref attach

`POST /api/meetings/{id}/refs {thread_id}`: idempotent upsert, in one
transaction that validates the meeting exists and the thread exists and is
not archived (RR3-08). Client flow: create/locate thread → attach → send.
CLI reads never create refs.

### 7.3 Chips without ledger writes

The first message of "chat about this meeting" carries **server-stamped**
`meeting_ref` metadata (7.1 step 3 — never client-supplied). The
`garyx-models` reducer derives a `meeting_refs` field on the user-turn row
from committed metadata; clients dumb-render the chip from `render_state`.
A chip whose entity no longer exists renders in a missing/tombstone style
via the client's normal catalog lookup; since FK cascade removes refs on
entity deletion, the resolver does not report deleted entities (the
"entity: deleted" behavior from revision 3 is withdrawn, RR3-08).

## 8. UI surfaces (slice 3)

As revision 3 (galleries, 30 s visible polling, keyset-paged list, chips
via render_state, packaged validation).

## 9. Configuration and deployment prerequisites

As revision 3, plus: disabling `meeting_entities` stops new admissions and
unregisters the client; existing entities follow the per-state stalled
rules of 5.1 (joining aborts at its deadline; live stalls visibly;
finalizing finalizes at its deadline).

## 10. Failure and observability

As revision 3, plus `stalled_reason` in list output and logs, and
checkpoint-gap detection logged at boot repair.

## 11. Implementation impact map

Revision 3's map, plus: `garyx-bridge` `prepare_user_turn` choke point +
allowlist extension (`run_management.rs:94,254` area); write-time segment
splitting in the log writer; `index.bin` persistence at finalize; snapshot
continuation tokens on `/read`; keyset pagination on `/api/meetings`;
`--again` removed from CLI.

## 12. Test plan (delta per RR3-10)

- **Seam**: real `/api/chat/stream-input` HTTP/WS requests (and the
  desktop/iOS client paths in integration fixtures) assert the queued
  provider input contains the context block and the acknowledged user
  metadata carries the server-owned ref; caller-supplied ref/context
  metadata is dropped on every entry path.
- **Write-time splitting**: >32 KiB item becomes N independent seqs;
  fetch/confirm across the split; no sub-segment state anywhere.
- **Snapshot continuation**: live `--full` across pages pinned to one
  snapshot; token expiry; checksum mismatch; `--max-bytes` resume.
- **Confirm failures**: confirm-not-committed vs confirm-committed-
  response-lost (distinct assertions per 6.3).
- **Bounded ticks**: sustained `has_more=true` closes at each bound
  (pages/items/bytes/time) with ckpt per close; terminal signal serviced
  at the forced close; content readable between closes.
- **Arbitration**: EndedSignal + AbortRequest queued together → end wins;
  GraceExpired during finalizing accelerates completion; 10005 during
  finalizing completes early.
- **Stalled matrix**: joining stalled past deadline → aborts; live
  stalled (all three reasons) → never auto-aborts; finalizing stalled →
  finalizes at deadline; registered-but-auth-dead shows `auth_failed`.
- **Deletion**: terminal entity without content dir (joining-deadline
  abort) reads and deletes cleanly; tombstone reconcile ordered before log
  repair; all rename/commit/remove crash points.
- **Index/perf**: cold high-seq first read on a terminal entity via
  persisted index within budget; missing `index.bin` rebuild-once; boot
  does not scan terminal logs.
- **Refs**: attach validates thread live-ness in-transaction; attach vs
  thread-delete race; refs-query failure → retryable dispatch failure;
  entity/cursor failure → degraded stub; JSON-string encoding defeats
  CR/LF injection via topic.
- **List**: keyset pagination stability while live rows update.
- Scope: six-crate `--all-targets` + tier1 + SwiftPM + `xcodebuild` +
  packaged desktop (unchanged).

## 13. Delivery slices

As revision 3, with slice 1 additionally gated on the seam tests (bridge
`prepare_user_turn` on all entry paths) since the resolver ships in
slice 1.

## 14. Open questions (defaults chosen, non-blocking)

- Retention: manual deletion only.
- One entity per admission (explicit product rule, §2).
- `meeting_activity_v1` ignored in Phase 1.
