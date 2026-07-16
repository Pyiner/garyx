# Feishu meeting entity

Status: revision 3, addressing adversarial review #TASK-2337 rounds 1–2
Author: gary (design)
Supersedes: `docs/design/feishu-group-listen-mode.md` (group listening product
surface is cancelled). Meeting **content** never enters thread ledgers; the
foreign-tail hazard that design documented is honored by shipping zero
background ledger writes in Phase 1 (Section 7.3).

## 1. Summary

When someone invites the Garyx bot into an ongoing Feishu meeting, the bot
joins, polls in-meeting events, and materializes a first-class **meeting
entity**. Live entities grow tick by tick; every poll tick durably closes
everything it produced before the watermark advances, so a crash never loses
captured content. When the meeting ends, capture drains the platform's grace
window and the entity becomes immutable.

Agents read entities only through `garyx meeting read`. The CLI resolves the
calling thread from `GARYX_THREAD_ID`; the server keeps a per-(entity,
thread) cursor with a **fetch/confirm two-phase protocol executed inside the
single CLI invocation**, so delivery is at-least-once (duplicates possible on
failure, silent skips impossible) while the agent experience stays "run one
command, get the increment, told exactly what it is".

All lifecycle transitions run through one per-entity coordinator over
durable CAS states with persisted intent stages, so every path — including
abort and finalize — is crash-resumable.

## 2. Product contract (user-approved, 2026-07-16)

1. Meetings only. No group-chat listening product surface.
2. Start: an in-meeting invite (`vc.bot.meeting_invited_v1`) admits, joins,
   and starts polling. Manual `garyx meeting join` is a debug path.
3. Live: the entity accumulates content in near-real-time (~30 s transcript
   latency is a platform property).
4. Read protocol: CLI-only content access; per-thread server cursors;
   incremental by default; self-describing output.
5. End: the meeting-ended signal stops capture and freezes the entity.
6. Reference continuity: threads that referenced an entity keep getting
   incremental reads; recognition is server state.

Product sign-off items (accepted limits):

- Feishu WS ACKs before processing. An invite is lost if the process dies,
  or the admission insert fails after bounded retries (3 attempts with
  backoff; disk-full/DB-error is logged at error level), in the window
  between ACK and durable admission. After admission everything is
  crash-recoverable.
- Re-inviting the bot to the same meeting after its entity reached a
  terminal state (or was deleted) creates a **new** entity capturing from
  that point on. One entity per admission, by design.
- A meeting's final ~30 s of speech becomes readable during the post-end
  grace drain, not instantly.

## 3. Verified repository baseline

B1–B12 as re-verified in review round 2 (all CONFIRMED there):

| # | Fact | Evidence |
|---|---|---|
| B1 | WS ACKs before processing; only `im.message.receive_v1` handled; 30 min event-id dedup cache | `garyx-channels/src/feishu/ws.rs:968,1040,1049-1072`, `feishu.rs:95` |
| B2 | `FeishuChannel::new(config, router, bridge, dispatcher, public_url)` → `FeishuRuntimeContext` | `feishu.rs:370`, `ws.rs:38-69` |
| B3 | gateway depends on channels (never reverse); a trait defined in channels and implemented in gateway compiles. This design introduces the **first** gateway→channels injected trait, threaded like `dispatcher` | `garyx-gateway/Cargo.toml:20`, `plugin.rs:3235,3322` |
| B4 | `FeishuClient` is `Clone` but `pub(crate)` — gateway cannot name it; cross-crate use requires a public trait object | `client.rs:127` |
| B5 | `CronService`: select loop, stop channel, `Weak<AppState>`, stale-state reset on boot | `cron.rs:633,713-796,799-851` |
| B6 | Capsule precedent: disk content + SQLite STRICT metadata + PRAGMA migration (storage shape only) | `capsules.rs:158,273`, `garyx_db/mod.rs:2645,3559` |
| B7 | Control records are produced inside provider-run persistence ordering; foreign rows at a run tail trigger reconcile re-append — no safe background control seam | `persistence.rs:364`, `store.rs:521` |
| B8 | `GARYX_THREAD_ID` injected into agent env from runtime metadata | `gary_prompt.rs:148-154`, `commands/task.rs:233` |
| B9 | CLI mutation timeout 10 s — all reads must fit a page budget | `gateway_client.rs:15` |
| B10 | `gary_prompt` is a synchronous formatter; meeting context must be resolved earlier and carried in metadata | `gary_prompt.rs:36,73,125` |
| B11 | Per-thread SSE forwards only matching `committed_message`; no global client event route → no meeting SSE in Phase 1 | `event_stream_hub.rs:6-49`, `routes.rs:2369` |
| B12 | Thread delete/archive are their own transaction paths; the workflow purge is one-shot, not a hook | `garyx_db/mod.rs:650,2036` |
| B13 | The one seam every user-turn entry path (channel, app/API, queued follow-up, internal dispatch) truly shares is bridge `start_agent_run`, including the queued branch | `run_management.rs:439,491` |

External platform facts: as revision 2 (join/events endpoints, `page_token`
continuation documented by the official manual, event types, ~30 s
transcript latency in 5 s/100-item batches, multi-party only, per-meeting
owner switch, `10005` not-in-meeting, 5-minute post-end window, `20001`
after it, minutes not auto-authorized, console event subscription
prerequisite). `page_token` is treated as an **opaque** continuation token:
never compared, never ordered, only stored and resumed from (RR2-01).

### 3.1 Sample-pinning gate

Slice 2 starts by capturing sanitized real samples (invite/ended envelopes,
join response, events pages) and committing them as fixtures. Two facts must
be pinned before ingestion code: (a) invite payload fields and the identity
usable at joining time; (b) whether the ended event's meeting identity can
be correlated to a joining-stage entity (Section 5.4 end-during-joining).
A mismatch with this design stops slice 2 for a design amendment. Slice 1
has no Feishu dependency.

## 4. Entity model and storage

### 4.1 SQLite

`meetings` (STRICT; all timestamps UTC RFC3339 `Z`):

```
id                 TEXT PRIMARY KEY
account_id         TEXT NOT NULL
meeting_no         TEXT NOT NULL
feishu_meeting_id  TEXT NOT NULL DEFAULT ''
invite_event_id    TEXT NOT NULL
call_id            TEXT NOT NULL DEFAULT ''
topic              TEXT NOT NULL DEFAULT ''
invited_by         TEXT NOT NULL DEFAULT ''
status             TEXT NOT NULL CHECK(status IN
                     ('joining','live','finalizing','aborting','finalized','aborted'))
status_detail      TEXT NOT NULL DEFAULT ''
end_source         TEXT NOT NULL DEFAULT '' -- 'push' | 'poll_ended' | 'grace_expired' | ''
join_deadline_at   TEXT NOT NULL
grace_deadline_at  TEXT
poll_cursor        TEXT NOT NULL DEFAULT ''  -- cache; checkpoint chain in the log is truth
closed_segment_count INTEGER NOT NULL DEFAULT 0 -- checkpoint cache; log is truth
byte_size          INTEGER NOT NULL DEFAULT 0
started_at         TEXT NOT NULL
ended_at           TEXT
finalized_at       TEXT
created_at / updated_at TEXT NOT NULL
```

Uniqueness (RR2-05, RR2-11):

- `UNIQUE(invite_event_id)` — WS redelivery idempotence.
- Partial unique `(account_id, meeting_no) WHERE status IN
  ('joining','live','finalizing','aborting')` — at most one **active**
  entity per meeting per account. Terminal/deleted entities do not block a
  fresh admission: one entity per admission is the product rule (§2).
- Partial unique `(account_id, feishu_meeting_id) WHERE
  feishu_meeting_id <> '' AND status IN
  ('joining','live','finalizing','aborting')` — long-id variant, empty
  strings excluded.

`meeting_read_cursors` (STRICT):

```
meeting_id     TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE
thread_id      TEXT NOT NULL
confirmed_seq  INTEGER NOT NULL DEFAULT 0
pending_from   INTEGER
pending_to     INTEGER
receipt        TEXT              -- opaque token for the pending span
updated_at     TEXT NOT NULL
PRIMARY KEY (meeting_id, thread_id)
CHECK ((pending_from IS NULL) = (pending_to IS NULL)
   AND (pending_from IS NULL) = (receipt IS NULL)
   AND (pending_from IS NULL OR
        (pending_from > confirmed_seq AND pending_to >= pending_from)))
```

`meeting_thread_refs` (STRICT) — a **canonical relation**, not a projection:
the ref is created by an explicit attach API and this table is its single
source of truth (mirroring `thread_channel_endpoints`' standing). Condition
queries about refs therefore read this table directly, satisfying the
no-body-scan rule.

```
meeting_id  TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE
thread_id   TEXT NOT NULL
attached_at TEXT NOT NULL
PRIMARY KEY (meeting_id, thread_id)
```

Indexes: `idx_meetings_updated(updated_at DESC)`,
`idx_meetings_status(status)`, `idx_refs_thread(thread_id, attached_at DESC)`.

Thread delete/archive transactions (B12) additionally
`DELETE FROM meeting_read_cursors / meeting_thread_refs WHERE thread_id=?`;
`/read` validates thread existence inside the cursor transaction so deleted
threads cannot resurrect cursors.

### 4.2 Content: checkpointed segment log (RR2-01)

`~/.garyx/meetings/{entity_id}/segments.jsonl` is the single content truth.
Two line kinds:

```
{"t":"seg","seq":12,"kind":"transcript","speaker":"张三","start":"…","end":"…",
 "text":"…","sources":["sent_8813","sent_8814"]}
{"t":"ckpt","cursor_out":"pt_x9y8…","at":"2026-07-16T02:35:12Z"}
```

**Tick protocol (the crash-safety core):**

1. A poll tick pages from the persisted cursor until `has_more=false`,
   pausing between pages to service terminal signals (Section 5.4).
2. All items produced by the tick are **closed at tick end** — speaker
   coalescing operates within a single tick only; nothing stays open across
   ticks, so there is no open-segment durability problem by construction.
   (Cost: an utterance spanning two ticks becomes two segments. Correctness
   over cosmetics.)
3. Write order, under the entity I/O lock: append all `seg` lines → append
   one `ckpt` line with the tick's final `cursor_out` (written for **every**
   tick, including empty or all-duplicate ones) → `fdatasync` the file →
   update SQLite caches (`poll_cursor`, `closed_segment_count`,
   `byte_size`, `updated_at`) in one transaction.
4. Dedup on replay/re-pull is by `sources` platform ids (sentence ids, chat
   message ids, share event ids) — re-materializing a re-pulled page is a
   no-op.

**Boot repair:** validate the log line by line, truncating a torn tail; the
last valid `ckpt` line is the authoritative resume cursor (SQLite
`poll_cursor` is only a cache and is corrected from the log); `seg` lines
after the last `ckpt` (crash mid-step-3) are kept — re-pulling their page is
idempotent by `sources`. Segment count/byte size recompute from the log.
Opaque tokens are never compared for freshness — the chain position defines
recency (RR2-01).

Markdown is a render format produced from structured segments at response
time; no Markdown lives on disk. There are no header or end-marker segments
— entity metadata renders headers/footers (RR2-02 simplification).

### 4.3 Entity deletion (RR2-07)

Only terminal entities are deletable. Under the entity I/O lock:

1. atomically `rename {id}/ → {id}.tombstone/`;
2. DB transaction deletes the `meetings` row (cascading cursors/refs);
3. remove the tombstone dir.

Boot sweep: `tombstone` **with** a surviving row → rename back (delete
never happened); `tombstone` without a row → remove it; an ordinary dir
without a row (legacy orphan) → logged and removed. Every crash point lands
in one of these three recoveries.

API: `DELETE /api/meetings/{id}` (409 for non-terminal entities) + CLI
`garyx meeting delete <id>`.

## 5. Ingestion

### 5.1 Platform client seam and registry lifecycle (RR2-06)

`garyx-channels` defines (new `meeting_sink.rs`):

```rust
#[async_trait]
pub trait MeetingPlatformClient: Send + Sync {
    async fn join(&self, meeting_no: &str, call_id: Option<&str>) -> Result<JoinedMeeting, MeetingApiError>;
    async fn poll_events(&self, feishu_meeting_id: &str, page_token: &str) -> Result<EventsPage, MeetingApiError>;
}

pub trait MeetingEventSink: Send + Sync {
    fn register_client(&self, account_id: &str, client: Arc<dyn MeetingPlatformClient>);
    fn unregister_client(&self, account_id: &str);
    fn on_meeting_invited(&self, invite: MeetingInvite);
    fn on_meeting_ended(&self, account_id: &str, feishu_meeting_id: &str);
}
```

- `FeishuChannel::start` registers one adapter per enabled account (adapter
  lives in garyx-channels where `FeishuClient` is nameable); `stop` and
  account disable/removal unregister; re-register replaces (restart-safe).
- A coordinator that finds no client for its account **stays in its
  persisted state** and retries every 60 s; `garyx meeting list` marks such
  entities `stalled (no platform client)`. Permanent situations (account
  deleted, `meeting_entities=false`, dead credentials) are resolved by the
  administrative terminator `garyx meeting abort <id>` /
  `POST /api/meetings/{id}/abort`, which runs the normal abort path. This
  cleanly distinguishes startup races (self-heal) from permanent loss
  (explicit, auditable action).
- `on_meeting_invited` performs the admission insert (bounded: 3 attempts
  with 100 ms/1 s backoff) plus a coordinator nudge, and nothing else — no
  network, no long waits on the WS loop. Persistent insert failure is
  logged at error level (product sign-off, §2).
- `MeetingApiError` is typed: `NotInMeeting`, `MeetingEnded`,
  `RetriableTransport`, `Other(code, msg)`.

### 5.2 Per-entity coordinator over durable intents (RR2-02)

`MeetingService` (gateway; CronService shape) runs one coordinator task per
non-terminal entity, consuming a command queue (`Nudge`, `EndedSignal`,
`AbortRequest`, `PollTick`, `Shutdown`). Every transition is a durable CAS
on `status`; CAS losers are no-ops. **Terminal states are reached only
through persisted intent stages**, so every flush is resumable:

- **finalize**: `live → finalizing` (intent; drain runs here) →
  [drain complete] → `finalizing → finalized` (pure CAS, no I/O between
  drain end and CAS; the drain's last tick already checkpointed).
- **abort**: `joining|live → aborting` (intent, with `status_detail`) →
  flush: close nothing (segments only close at tick end; there is nothing
  open between ticks), write final `ckpt` if a tick was interrupted →
  `aborting → aborted`.
- Boot resumes `finalizing` (drain until `grace_deadline_at`) and
  `aborting` (finish flush, CAS terminal) exactly where they stopped.
- `finalized` and `aborted` both refuse appends (writer checks status under
  the I/O lock).
- Priority: once `finalizing` is entered, `AbortRequest` is refused
  (drain owns the endgame); `NotInMeeting` during finalizing means no more
  data is readable → complete the drain early and CAS to `finalized`
  (RR2-02, RR2-04). The administrative abort (5.1) applies to
  `joining|live|stalled-finalizing` where stalled means no client.

**Admission (K1):** the sink's insert creates
`status='joining'` with `join_deadline_at = now + join_retry_window`
(absolute). Unique indexes make duplicate invites no-ops (§4.1). The
content dir is created lazily by the writer (`ensure_dir` before first
append — idempotent, self-healing for a missing dir; no file work sits
between any CAS pair, RR2-02).

**Joining:** retry `join` every 20 s until success or the absolute
deadline. Success: CAS `joining → live` + backfill `feishu_meeting_id`,
`topic`. Deadline: abort path (entity remains as the durable record).
Restart: resume retrying to the same deadline. If the meeting had already
ended, `join` keeps failing and the deadline aborts — acceptable; and if
join succeeds during the grace window, the first poll returns
`MeetingEnded` and the normal end path runs (this also covers
"end-during-joining": no cross-identity correlation is needed because the
poll bootstraps the end, RR2-04).

**Live:** `PollTick` every `poll_interval_secs` (default 30, jittered),
executing the tick protocol of §4.2. Between pages the coordinator services
its queue; a terminal signal interrupts pagination after the current page
completes its checkpointed write (RR2-04 starvation fix). Transport errors
back off 30→60→120 s. `NotInMeeting` with no end signal → abort path (bot
removed). `MeetingEnded` → end path.

### 5.3 End path and grace drain (RR2-04)

On `EndedSignal(source)`:

- `source=push` or `poll_ended` (API returned MeetingEnded): CAS
  `live → finalizing`, `ended_at = now`,
  `grace_deadline_at = now + 4 min`, `end_source` recorded. Drain: keep
  polling on normal cadence **until the deadline** — no quiescence
  shortcut; the platform gives no completeness watermark, so we spend the
  window we are given (RR2-04). Then CAS to `finalized`.
- `source=grace_expired` (`20001`): the platform says the window is over —
  nothing to drain. CAS `live → finalizing` (recording `end_source`) and
  immediately complete: final `ckpt`, CAS `finalized`.
- `EndedSignal` for an unknown `(account_id, feishu_meeting_id)` — e.g. the
  entity is still `joining` with an empty long id — is logged and dropped;
  the joining→live→first-poll bootstrap covers it (5.2).
- Duplicate/racing signals lose the `live → finalizing` CAS and vanish.

### 5.4 Lifecycle state machine

```
 invite ──admission insert (unique invite_event_id;
    │      unique active (account,meeting_no))──> JOINING
    │                                               │  join ok (CAS)
    │              deadline exhausted (→ABORTING)   v
    │                                             LIVE ──PollTick: paged pull,
    │                                               │      close-at-tick-end,
    │                                               │      seg*+ckpt+fdatasync,
    │                                               │      SQLite cache update
    │       10005 while live (→ABORTING)            │
    │                                               │ EndedSignal(push|poll_ended)
    │                                               │  (CAS live→finalizing)
    v                                               v
 ABORTING ──flush ckpt──> ABORTED           FINALIZING ── drain to grace_deadline_at
 (intent)              (terminal)            (intent)      (20001 ⇒ complete now;
                                                 │          10005 ⇒ complete early;
                                                 │          abort refused here)
                                                 v  (CAS)
                                            FINALIZED (terminal)

 boot: JOINING/LIVE resume their loops; FINALIZING resumes drain to its
 persisted deadline; ABORTING finishes flush→terminal. Missing client ⇒
 stalled + retry, admin abort available. Terminal rows never spawn
 coordinators; reads/deletes go through the entity I/O lock (6.2).
```

### 5.5 Restart recovery

Boot order: repair logs (4.2) → sweep tombstones (4.3) → spawn coordinators
for all non-terminal rows into their persisted stage. No in-flight memory is
needed: deadlines are absolute columns, the resume cursor is the log's
checkpoint chain, and clients arrive via the startup registry.

## 6. Read protocol

### 6.1 CLI surface

- `garyx meeting list [--json]` — paged (`limit` 50 default,
  server-paginated for unbounded retention, RR2-08); shows id, topic,
  status (+`stalled` marker), closed segments, updated_at.
- `garyx meeting read <entity_id> [--full] [--range A..B] [--again]
  [--thread <id>] [--json]`
- `garyx meeting join <meeting_no> [--account <id>]` — debug.
- `garyx meeting abort <id>` / `garyx meeting delete <id>` — admin.

Identity rules: `incremental` (default) requires thread identity
(`GARYX_THREAD_ID` or `--thread`; missing both errors with both remedies).
`--full` and `--range` are **stateless paged peeks**: no thread identity
needed, no cursor interaction ever (RR2-03 simplification: full no longer
touches cursors; a thread that wants "caught up to everything" reads
increments to exhaustion, which the header makes a short loop).

### 6.2 Gateway API and locking

```
GET    /api/meetings?limit&page_token        -> paged list
GET    /api/meetings/{id}                    -> metadata
POST   /api/meetings/{id}/read               -> fetch (incremental|full|range)
POST   /api/meetings/{id}/read/confirm       -> confirm {receipt}
POST   /api/meetings/{id}/refs               -> attach ref {thread_id} (idempotent upsert)
POST   /api/meetings/{id}/abort              -> admin abort
DELETE /api/meetings/{id}                    -> delete (terminal only)
POST   /api/meetings/{id}/join-debug         -> manual trigger
```

**Entity I/O lock (RR2-08):** a per-entity `RwLock` in a service-level map
covers every state including terminal: writers (tick appends, finalize
flush, delete rename) take write; reads take read and capture a snapshot
`(closed_latest, log_byte_offset)` so slicing never sees torn appends;
delete vs read is serialized by the same lock. Live entities' snapshots are
requested through the coordinator; terminal entities take the lock
directly.

**Offset index (RR2-08):** an in-memory sparse index (every 64th segment →
byte offset), built lazily on first read and extended per append, makes
high-seq slicing O(segment span), not O(file). Rebuilt from the log on
demand; never persisted.

**Oversized segments:** a rendered response is capped (default 64 KiB /
200 segments; `page_bytes` may lower). A single segment larger than the
byte cap is split at UTF-8 boundaries into deterministic continuation
chunks labeled `[seq 12 part 2/3]`; pagination state is
`(seq, part)`-addressed so the budget holds for any input (RR2-08).

### 6.3 Fetch/confirm two-phase (RR2-03)

Cursor state: `confirmed_seq` + `pending(from,to)` + `receipt` (opaque,
server-generated per fetch).

- **fetch** (`/read`, incremental): if a pending span exists → re-serve
  exactly that span with its stored receipt (at-least-once). Else slice
  `(confirmed_seq, min(latest, page cap)]`, persist it as pending with a
  fresh receipt, serve it. Fetch **never** advances `confirmed_seq`.
- **confirm** (`/read/confirm {receipt}`): CAS guarded by
  `(confirmed_seq, pending_from, pending_to, receipt)` — advances
  `confirmed_seq = pending_to`, clears pending. Unknown/stale receipt is a
  no-op success (idempotent).
- **The CLI performs both inside one invocation**: fetch → render to
  stdout → flush successfully → confirm → exit. A crash or broken pipe
  before flush leaves pending un-confirmed, so the span re-serves next
  time. A lost confirm response leaves pending; the next fetch re-serves
  the same span (duplicate, never a skip). `--again` re-serves pending and
  skips the confirm call.
- The agent-visible experience is unchanged: run the command, get content;
  re-running after a failure repeats the last chunk. The header states
  confirmation status explicitly.

Cursors never regress; concurrent reads from one thread serialize on the
row CAS, stale writers lose harmlessly.

### 6.4 Self-describing output

```
── meeting entity 019f… ─ 《Q3 规划会》 ─ LIVE, updated 10:35:12 ──
Increment for this thread: segments 12–18 of 23 closed (5 more after this)
Delivered & confirmed on success; if this command failed midway, rerunning
re-serves the same span.  Re-serve without confirming: --again
Everything (stateless, paged): --full     Peek a span: --range 5..9
────────────────────────────────────────────
[12] 10:32:05 张三 (transcript)
…
```

Finalized/aborted entities state so with timestamps, `end_source`, and
abort reason. Empty increments return the header plus "no new segments
since [confirmed 18]". Live entities note the meeting is in progress.

## 7. Pointer injection and references

### 7.1 Resolver in the bridge (RR2-09)

The only seam all four entry paths share is bridge `start_agent_run`
(B13). Therefore:

- `garyx-bridge` defines `#[async_trait] MeetingContextResolver` (bridge
  cannot depend on gateway; gateway implements and injects at assembly,
  like other gateway-owned services).
- `start_agent_run` invokes it inside the thread dispatch guard, before
  provider resolve and before the queued-input decision, for both fresh and
  queued turns. It point-reads `meeting_thread_refs` (newest 3 by
  `attached_at`, documented cap), entity metadata, and this thread's cursor.
- The result is written into run metadata as a **server-owned** field: any
  caller-supplied `meeting_context` metadata is discarded and overwritten
  (spoof-proof). Topic and all platform-derived text are XML-escaped and
  length-capped (topic 200 chars) before formatting.
- Failure semantics: if the resolver errors, it degrades to a stub block
  listing entity ids only, marked `context: unavailable (retryable)` — the
  agent still learns the refs exist and can use the CLI; the block is never
  silently omitted when refs exist (RR2-09).
- `gary_prompt` stays synchronous: it formats the metadata value into
  `<garyx_meeting_context>` beside `<garyx_thread_metadata>`.

### 7.2 Ref attach (RR2-10)

`POST /api/meetings/{id}/refs {thread_id}` is an idempotent upsert into the
canonical `meeting_thread_refs` relation (4.1). The desktop/iOS "chat about
this meeting" action: create/locate the thread (existing APIs) → attach ref
→ send the first message. Attach-then-send makes a crash between the two
harmless (a ref without a message just injects context on the next turn).
CLI reads never create refs.

### 7.3 Chips without ledger writes (RR2-10)

Phase 1 writes **no** background rows into thread ledgers (B7 hazard). The
"chat about this meeting" first message carries
`meeting_ref: {entity_id}` in its user-message metadata; the
`garyx-models` render-state reducer derives a `meeting_refs` field on the
user turn row from committed metadata, and desktop/iOS dumb-render the chip
from `render_state` per the transcript contract — no client-side metadata
parsing, no new control kinds. A deleted entity renders the chip in a
disabled/tombstone style resolved at render time by the client's normal
catalog lookup; the resolver likewise reports `entity: deleted` in the
context block if a ref outlives its entity.

## 8. UI surfaces (slice 3)

- Desktop gallery ("Meetings", left rail): refresh on open + 30 s polling
  while visible; paged list API. Actions: chat-about (7.2), admin abort for
  stalled entities, delete for terminal. No SSE in Phase 1 (B11).
- iOS: catalog via stale-while-refresh; logic in `GaryxMobileCore` with
  SwiftPM tests; packaged `xcodebuild` validation.
- Chip rendering via render_state only (7.3).

## 9. Configuration and deployment prerequisites

- `FeishuAccount.meeting_entities: bool` (`default_true`). Disabling stops
  new admissions and unregisters the client; existing non-terminal entities
  become `stalled` and are resolved by admin abort (5.1).
- `GatewayConfig.meetings: MeetingConfig` — `poll_interval_secs` (30, clamp
  10–120), `join_retry_window_secs` (300), `read_page_bytes` (65536).
- Developer console (blocks slice 2): subscribe `vc.bot.meeting_invited_v1`
  + `vc.bot.meeting_ended_v1`, publish app version. Scopes already live.
- Fixtures sanitized; no real ids anywhere.

## 10. Failure and observability

- Lifecycle transitions log info (entity, feishu id, CAS from→to, source).
- Typed poll/join errors with backoff state; `status_detail` and
  `end_source` surface in `garyx meeting list --json`.
- Boot repair logs truncated bytes, replayed lines, cursor corrections,
  tombstone dispositions.

## 11. Implementation impact map

As revision 2, plus: `meeting_sink.rs` gains `unregister_client`;
`garyx-gateway/src/meetings/` gains the I/O-lock map, offset index, confirm
route, refs route, abort/delete routes; `garyx-bridge` gains the
`MeetingContextResolver` trait + `start_agent_run` call; `garyx-models`
gains the user-turn `meeting_refs` render field; CLI gains
`confirm` flow, `--again`, `abort`, `delete`.

## 12. Test plan (delta on top of revision 2's matrix, per RR2-12)

**Storage/watermark:** crash after seg lines before ckpt (re-pull dedups);
crash after ckpt before SQLite (cache corrected from log); empty and
dedup-only ticks still checkpoint; cross-tick utterance split; opaque-token
resume (never compared); torn-line truncation; fdatasync ordering asserted
via fault injection.

**Lifecycle:** join CAS with no dir (lazy ensure_dir); end-during-joining
via join-succeeds-then-first-poll-ended fixture and via join-fails-to-
deadline; EndedSignal(push) vs poll_ended race (CAS loser no-op); `20001`
immediate finalize; `10005` during finalizing completes early; AbortRequest
refused in finalizing; abort intent crash-resume (`aborting` on boot);
transcript arriving after two empty pulls but before deadline is captured
(no quiescence); pagination interrupted by terminal signal at a page
boundary; distinct-event reinvite after terminal and after delete creates a
fresh entity; admission insert failure path (bounded retries, error log).

**Registry:** unregister on stop/disable; replace on re-register; stalled
marker; admin abort of stalled entities; startup race (client arrives after
recovery began).

**Deletion:** crash between rename and DB commit (tombstone+row → restore);
between commit and tombstone removal (tombstone, no row → sweep); legacy
ordinary orphan sweep; delete vs concurrent read under the I/O lock; 409 on
non-terminal delete.

**Read protocol:** fetch/confirm sequences with dropped fetch response,
dropped confirm response, crash between render and confirm (span
re-served); `--again` never confirms; receipt CAS with stale receipt;
concurrent incremental + `--full` (full is stateless, cursor untouched);
high-confirmed thread running `--full` from 1 (no cursor effect); oversized
segment split determinism; high-seq read via offset index (perf bound);
list pagination.

**Resolver/render:** resolver on all four entry paths (channel, app, queued,
internal); degraded stub block on resolver failure (never omitted with refs
present); caller `meeting_context` metadata discarded; XML escape of
hostile topic; reducer derives `meeting_refs` from committed metadata;
clients dumb-render (contract test against render_state snapshot); chip for
deleted entity.

**Scope:** `cargo test --all-targets` for `garyx-channels`, `garyx-gateway`,
`garyx-bridge`, `garyx-models`, `garyx-router`, and `garyx` (CLI); tier1
fast loop; slice 3 adds SwiftPM + `xcodebuild` + packaged desktop check.

## 13. Delivery slices

1. **Entity core + read protocol** (no Feishu): tables, checkpointed log +
   repair, I/O lock + offset index, fetch/confirm cursors, list/read/
   delete CLI + routes, resolver + context block + reducer field, fixture
   tests. Gate: read-protocol, storage-fault, and deletion suites green;
   hand-seeded entity readable incrementally from two threads with
   independent receipt-confirmed cursors, including forced-failure
   re-serves.
2. **Ingestion**: sample capture gate (3.1) → event structs, sink +
   registry lifecycle, coordinator with intent CAS lifecycle, tick
   protocol, grace drain, recovery, admin abort, console subscriptions.
   Gate: real meeting live→finalizing→finalized with correct segments;
   restarts mid-joining/mid-live/mid-finalizing/mid-aborting all resume;
   lifecycle suite green.
3. **Experience**: galleries, ref attach + chips, polling refresh. Gate:
   card → chat → agent reads increment → answer cites new segments, end to
   end on packaged desktop + simulator.

## 14. Open questions (defaults chosen, non-blocking)

- Retention: manual deletion only.
- One entity per admission (terminal re-invites create new entities) — now
  an explicit product rule (§2).
- `meeting_activity_v1` ignored in Phase 1.
