# Feishu meeting entity

Status: revision 2, addressing adversarial review #TASK-2337 (R1–R15)
Author: gary (design), based on #TASK-2322 research and two seam surveys
Supersedes: `docs/design/feishu-group-listen-mode.md` (group listening product
surface is cancelled). Its passive-transcript machinery is not used for
meeting **content**; Section 7.3 explains why background ledger writes are
nevertheless deferred out of Phase 1 (the tail-reconciliation hazard that
design documented still applies to any foreign row).

## 1. Summary

When someone invites the Garyx bot into an ongoing Feishu meeting, the bot
joins, polls in-meeting events, and materializes a first-class **meeting
entity**. While the meeting is live the entity grows incrementally
(transcript utterances, in-meeting chat, share events). When the meeting ends
the entity finishes a grace-window drain and is **finalized**, immutable.

The user can open a conversation about the entity at any time. The agent
never receives meeting content inline and never receives a file path. It
receives a **pointer**: entity id, status, metadata, and one line of CLI
usage. Content is read exclusively through `garyx meeting read`, which
resolves the calling thread from `GARYX_THREAD_ID`, keeps a per-(entity,
thread) cursor **on the server**, returns the unread increment by default
with at-least-once delivery (receipt-confirmed, never silently skipped), and
self-describes its output.

Meeting content lives outside every thread transcript; the entity is its own
source of truth. All entity lifecycle transitions are owned by one
per-entity coordinator over durable CAS states, so invites survive restarts,
end/abort paths are idempotent, and recovery needs no in-flight state.

## 2. Product contract (user-approved, 2026-07-16)

1. Meetings only. No group-chat listening product surface.
2. Start: an in-meeting invite (`vc.bot.meeting_invited_v1`) joins the bot
   and starts polling. Manual `garyx meeting join` exists as a debug path.
3. Live: the entity accumulates content in real time.
4. Read protocol: agents MUST read entity content through the CLI. The
   entity records which thread has read what; repeat reads return the
   increment by default; output states what it is and how to get the rest.
5. End: the meeting-ended signal stops capture and freezes the entity.
6. Reference continuity: a thread that referenced the entity before gets
   incremental reads by default — recognition is server state, not agent
   memory.

Accepted delivery limits (product sign-off items):

- Feishu's WS transport ACKs before processing (at-most-once). If the
  process dies between ACK and the durable invite enqueue (a few
  milliseconds), that invite is lost. After the enqueue, everything is
  recoverable. This is the same acceptance the transport already imposes on
  chat messages.
- Transcript items arrive ~30 s late in batches; a meeting's last words
  become readable only during the grace drain after it ends.

## 3. Verified repository baseline

Re-verified for revision 2; review #TASK-2337 corrections incorporated.

| # | Fact | Evidence |
|---|---|---|
| B1 | Feishu WS dispatch handles only `im.message.receive_v1`; events are ACKed before processing and deduped by `event_id` in a 30 min in-memory cache | `garyx-channels/src/feishu/ws.rs:968,1040,1049-1072` |
| B2 | `FeishuChannel::new(config, router, bridge, dispatcher, public_url)`; injected deps flow into `FeishuRuntimeContext` | `feishu.rs:370`, `ws.rs:38-69` |
| B3 | Crate direction permits "trait defined in channels, implemented by gateway" (gateway depends on channels). There is **no existing** gateway→channels injected trait; `ThreadCreator` is gateway→router. This design introduces the first one, threaded like `dispatcher` through `with_dispatcher` | `garyx-channels/src/plugin.rs:3235,3322`, `garyx-router/src/router/contracts.rs:21` |
| B4 | `FeishuClient` owns tenant-token refresh and is `Clone`, but it is `pub(crate)` — **not nameable from gateway**. Cross-crate access requires a public trait object (Section 5.1) | `client.rs:127,226,243` |
| B5 | Background-service precedent: `CronService` (select loop, stop channel, `Weak<AppState>`, stale-running reset on boot) | `cron.rs:633,713-796,799-851` |
| B6 | Storage precedent: capsules (disk content + SQLite STRICT metadata + PRAGMA migration). Storage shape only; recovery logic is defined fresh in Section 4.3 | `capsules.rs:158,273`, `garyx_db/mod.rs:325,2645,3559` |
| B7 | `capsule_attached` control records are produced inside the provider-run persistence worker ordering; there is **no** seam for a background service to append controls safely. Foreign rows at a run tail trigger re-append in `plan_reconcile_run_records_tail` | `persistence.rs:364`, `garyx-router/src/thread_history/store.rs:521` |
| B8 | `GARYX_THREAD_ID` injected into agent env when runtime metadata exists; CLI reads via `env_nonempty` | `gary_prompt.rs:148-154`, `commands/gateway_client.rs:439`, `commands/task.rs:233` |
| B9 | CLI→gateway: `gateway_endpoint` + bearer + retrying JSON helpers; mutation timeout is 10 s — long reads must paginate | `gateway_client.rs:15,73,101,220,294-387` |
| B10 | Prompt context blocks are built synchronously from run metadata by `gary_prompt`; the bridge has no DB access. Meeting context must therefore be resolved **before** dispatch and carried in metadata (Section 7.1) | `gary_prompt.rs:36,73,125` |
| B11 | `EventStreamHub` broadcasts strings, but per-thread SSE forwards only matching `committed_message`; there is no global client event route. Phase 1 therefore ships **no** meeting SSE (Section 8) | `event_stream_hub.rs:6-49`, `routes.rs:2379` |
| B12 | Thread deletion/archive transactions do not run the workflow purge; cursor/ref cleanup must hook the real thread delete path with FK cascades | `garyx_db/mod.rs:2036,2702` (delete/archive), `:2797` (one-shot workflow purge only) |

External platform facts (lark-cli 1.0.70 + internal onboarding manual; scopes
verified live on the production app):

- `POST /open-apis/vc/v1/bots/join` takes `meeting_no` (+ optional `call_id`
  from the invite) and returns the long numeric `meeting.id`. Tenant token.
- `GET /open-apis/vc/v1/bots/events`: pull, 10–30 s cadence, `page_token`
  pagination (20–100/page). The manual's guidance is explicit that the last
  `page_token` is kept and **continued from on the next incremental pull**
  — the token is the durable cross-tick watermark (Section 5.3).
- Event types: `participant_joined/left`, `chat_received`,
  `transcript_received`, `magic_share_started/ended` (`share_doc.title/url`).
- Transcript ~30 s latency, 5 s/100-item batching. Multi-party meetings
  only. Owner must enable "allow agents to join" per meeting. Bot must be in
  the meeting (`10005`); 5-minute post-end grace window (`20001` after).
- Post-meeting minutes are not auto-authorized to the bot (platform fix in
  progress) — not a dependency.
- `vc.bot.meeting_invited_v1` / `vc.bot.meeting_ended_v1` must be subscribed
  in the developer console (deployment prerequisite).

### 3.1 Sample-pinning gate (R14)

Exact invite/ended payload field names, the invite→`meeting.id` identity
mapping, and events-page shapes are pinned from sanitized captured samples
**before any ingestion code**: slice 2 begins with a capture step (invite the
bot to a scratch meeting; record raw WS envelopes and `bots/*` responses;
commit sanitized fixtures). The identity rule below (Section 5.2, K1) is
written against the manual's field set and re-validated at capture time; a
mismatch stops slice 2 for a design amendment, not an ad-hoc code decision.
Slice 1 (entity core + read protocol) has no Feishu dependency and proceeds
in parallel.

## 4. Entity model and storage

Metadata + all coordination state in SQLite; content in one structured
append log per entity. Gateway owns both.

### 4.1 SQLite

`meetings` (STRICT):

```
id                 TEXT PRIMARY KEY      -- uuid_v7 entity id
account_id         TEXT NOT NULL
meeting_no         TEXT NOT NULL         -- 9-digit number from the invite
feishu_meeting_id  TEXT NOT NULL DEFAULT '' -- long id; '' until join returns it
invite_event_id    TEXT NOT NULL         -- originating WS event id
call_id            TEXT NOT NULL DEFAULT ''
topic              TEXT NOT NULL DEFAULT ''
invited_by         TEXT NOT NULL DEFAULT ''
status             TEXT NOT NULL         -- 'joining'|'live'|'finalizing'|'finalized'|'aborted'
status_detail      TEXT NOT NULL DEFAULT '' -- abort reason / join failure detail
join_deadline_at   TEXT NOT NULL         -- absolute retry deadline (RFC3339)
grace_deadline_at  TEXT                  -- set on entering 'finalizing'
poll_cursor        TEXT NOT NULL DEFAULT '' -- last feishu page_token (watermark)
closed_segment_count INTEGER NOT NULL DEFAULT 0 -- derived checkpoint (log is truth)
byte_size          INTEGER NOT NULL DEFAULT 0   -- derived checkpoint
started_at         TEXT NOT NULL
ended_at           TEXT
finalized_at       TEXT
created_at / updated_at TEXT NOT NULL
```

Uniqueness (R1): `UNIQUE(invite_event_id)` makes the durable enqueue
idempotent against WS redelivery, and a partial unique index on
`(account_id, meeting_no) WHERE status IN ('joining','live','finalizing')`
prevents two concurrent entities for the same meeting regardless of how many
distinct invite events arrive. (`meeting_no` is the only identity known
before join; the long id backfills after join and gets its own partial
unique index over non-terminal rows.)

`meeting_read_cursors` (STRICT) — cursor plus delivery receipt (R7):

```
meeting_id     TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE
thread_id      TEXT NOT NULL              -- validated against thread_records at write
confirmed_seq  INTEGER NOT NULL DEFAULT 0 -- highest segment CONFIRMED delivered
pending_from   INTEGER                    -- last handed-out, unconfirmed span
pending_to     INTEGER
updated_at     TEXT NOT NULL
PRIMARY KEY (meeting_id, thread_id)
```

`meeting_thread_refs` (STRICT) — the SQL projection for "which threads hold
a ref" (R11; condition queries never scan record bodies,
`repository-contracts.md`):

```
meeting_id  TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE
thread_id   TEXT NOT NULL
attached_at TEXT NOT NULL
PRIMARY KEY (meeting_id, thread_id)
```

Ref attach writes and thread deletion cleanup happen in the same transaction
as their triggering record write. Thread delete/archive paths (B12) gain
`DELETE FROM meeting_read_cursors/meeting_thread_refs WHERE thread_id = ?` in
the same transaction; `/read` validates the thread exists inside its cursor
transaction so a deleted thread cannot resurrect a cursor row (R10).

### 4.2 Content: one canonical append log (R5, R6)

`~/.garyx/meetings/{entity_id}/segments.jsonl` — the **single content truth**
(new `default_meetings_dir()` beside `default_capsules_dir`). One JSON line
per **closed** segment:

```
{"seq":12,"kind":"transcript","speaker":"张三","start":"…","end":"…",
 "text":"…","sources":["sent_8813","sent_8814"],"batch":"pt_a1b2…"}
```

- `seq`: dense, monotonic, assigned **only at segment close** — an open
  (still-coalescing) utterance holds no seq and is invisible to cursors
  (R6). Coalescing state (the one open segment per entity) lives in
  coordinator memory; close triggers are speaker change, 60 s gap,
  non-transcript event, poll-tick boundary older than 90 s, meeting end. A
  crash loses at most the open segment's merge state; its source items
  re-materialize on replay of the persisted batch (below).
- `sources`: stable platform item ids (transcript `sentence_id`s, chat
  message ids, share event ids). Replay dedups on these, so a re-pulled or
  re-processed page cannot double-materialize content (R5).
- `batch`: the feishu `page_token` under which the items arrived, tying
  content lines to the poll watermark.
- Writes: append line + flush, then update SQLite checkpoint columns and
  `poll_cursor` in one transaction (write-then-derive). The log is always
  ≥ the checkpoint.
- **Boot repair** (R5): truncate a torn final line (validated JSON per
  line), replay the log tail to rebuild `closed_segment_count`, `byte_size`,
  and verify `poll_cursor`; if the log shows content from a batch newer than
  the stored `poll_cursor` (crash between file flush and SQLite commit),
  the stored cursor is behind — re-pulling that page is safe because
  `sources` dedup makes materialization idempotent.

There is no separate `events.jsonl` and no Markdown file on disk: raw
context needed for audit lives in the segment lines themselves; Markdown is
a **render format** produced by the CLI/API from structured segments, which
also removes the `## [999]` content-injection hazard against slicing (R6).

### 4.3 Entity deletion

Only terminal entities are deletable. Order: DB transaction deletes the
`meetings` row (cascading cursors/refs), commits, then the content dir is
renamed to `…/{id}.tombstone` and removed; a leftover tombstone dir is swept
on boot. A crash can leave a tombstone (cleaned next boot) but never a DB
row without its log or a readable entity without a row (R10).

## 5. Ingestion

### 5.1 Platform client seam (R4)

`garyx-channels` defines the public abstraction (gateway cannot name
`FeishuClient`, B4):

```rust
#[async_trait]
pub trait MeetingPlatformClient: Send + Sync {
    async fn join(&self, meeting_no: &str, call_id: Option<&str>) -> Result<JoinedMeeting, MeetingApiError>;
    async fn poll_events(&self, feishu_meeting_id: &str, page_token: &str) -> Result<EventsPage, MeetingApiError>;
}

pub trait MeetingEventSink: Send + Sync {
    fn register_client(&self, account_id: &str, client: Arc<dyn MeetingPlatformClient>);
    fn on_meeting_invited(&self, invite: MeetingInvite);   // durable enqueue ONLY, returns immediately
    fn on_meeting_ended(&self, account_id: &str, feishu_meeting_id: &str); // signal enqueue ONLY
}
```

- `FeishuChannel::start` registers a `MeetingPlatformClient` adapter (a thin
  wrapper over its `FeishuClient`, implemented inside garyx-channels where
  the type is nameable) for every enabled account **at channel startup** —
  not per invite. After a process restart the registry repopulates when
  channels start, before or shortly after `MeetingService` recovery begins;
  recovery of a non-terminal entity whose account client is not yet
  registered stays in its persisted state and retries on a timer — it is
  never aborted for a missing client (R4).
- Sink methods are non-blocking: `on_meeting_invited` performs one bounded
  SQLite insert (the `joining` row, idempotent on `invite_event_id`) and a
  coordinator nudge; join retries never run on the WS loop (R4). Errors are
  logged; the WS loop is never held beyond the insert.
- `MeetingApiError` distinguishes `NotInMeeting` (10005), `MeetingEnded`
  (20001), `RetriableTransport`, and `Other(code, msg)` so lifecycle
  decisions are typed, not string-matched.
- Threading: sink + registry handle injected through `with_dispatcher` →
  `FeishuChannel::new` → `FeishuRuntimeContext`, the same shape as
  `dispatcher` (B2/B3 — first gateway→channels trait, stated as such). A
  no-op sink keeps other assemblies/tests compiling.

### 5.2 Per-entity coordinator (R1, R2)

`garyx-gateway/src/meetings/` hosts `MeetingService` (CronService shape:
`Arc` on AppState, stop channel, `Weak<AppState>`). All lifecycle work runs
in **one coordinator task per entity** consuming a command queue; nothing
else mutates entity state. Commands: `Nudge` (invite enqueued / boot
recovery), `EndedSignal`, `PollTick` (self-scheduled), `Shutdown`.

Every transition is a durable CAS: `UPDATE meetings SET status = :to …
WHERE id = :id AND status = :from`; zero rows updated means another path
already owned the transition — the loser becomes a no-op. Both terminal
paths are idempotent and both leave the entity refusing appends
(`finalized` and `aborted` alike, R2).

**K1 — identity and admission.** The durable invite insert (from the sink)
is the admission point: `INSERT … status='joining'` guarded by the
`invite_event_id` unique index (WS redelivery) and the partial unique index
on live-ish `(account_id, meeting_no)` (distinct invite events for the same
meeting → second insert fails → dropped with a log line). `join_deadline_at
= now + join_retry_window` is written at insert time (absolute, restart-
safe).

**Joining:** the coordinator retries `join` every 20 s until success or
`join_deadline_at`. Success: CAS `joining→live`, backfill
`feishu_meeting_id`, `topic`, write header segment, start ticking. Deadline
exhausted: CAS `joining→aborted` with detail — the entity row remains as the
durable record that an invite was seen but never joined (R1: nothing is
silently lost after admission). Restart during joining: boot recovery
(Section 5.5) finds the row and resumes retrying until the same absolute
deadline.

**Live (PollTick, ~30 s, jittered):** `poll_events` from the persisted
`poll_cursor` watermark, paging until `has_more=false`; normalize items →
close/extend segments (Section 4.2) → append closed lines → CAS-free
checkpoint update (same transaction as `poll_cursor` advance). Transport
errors back off inside the tick schedule (30→60→120 s cap). `NotInMeeting`
while no ended-signal has arrived → **abort path** (bot removed
mid-meeting; single defined meaning of 10005 during live, R2).
`MeetingEnded` from the API → **end path**. Poll never finalizes directly;
it sends itself `EndedSignal`/`Abort` commands so the coordinator's CAS is
the only decision point.

**End path:** on `EndedSignal` (push event or poll-detected): CAS
`live→finalizing`, set `ended_at` and `grace_deadline_at = now + 4 min`
(inside the platform's 5-minute window). Duplicate signals lose the CAS and
vanish.

**Finalizing — the actual grace drain (R3):** keep polling on the normal
cadence until `grace_deadline_at` **or** two consecutive empty pulls at
least 60 s after `ended_at` (late transcript batches arrive for ~30 s+).
Then close any open segment, append one end-marker segment, CAS
`finalizing→finalized`, stop the tick. Restart during finalizing resumes
draining until the same persisted deadline (R3).

**Abort path:** CAS from `joining|live|finalizing → aborted` with
`status_detail` (removed from meeting / poll dead > 10 min / join deadline).
Open segment closes, marker appended, content stays readable.

### 5.3 Poll watermark semantics (R14)

The feishu `page_token` is the durable watermark: each tick resumes from the
stored token (manual-documented continuation semantics), pages to
`has_more=false`, and persists the final token in the same transaction as
the content it produced. Redelivered/overlapping pages are harmless:
materialization dedups on `sources` ids (4.2). Contract tests cover: new
events arriving mid-pagination, last page, resume-after-restart, duplicate
page replay (R14).

### 5.4 Lifecycle state machine

```
 invite event ──durable insert (K1: unique invite_event_id,
       │         unique live-ish (account, meeting_no))
       v
   JOINING ──join ok (CAS)──────────────> LIVE ◄─┐
       │                                    │     │ PollTick: pages from
       │ deadline exhausted (CAS)           │     │ watermark, closed
       v                                    │     │ segments appended
   ABORTED ◄──10005 while live / poll dead──┤     │
   (terminal,                               │ EndedSignal (push or poll; CAS)
    readable)                               v
                                       FINALIZING ── grace drain to deadline
                                            │        or quiescence (2 empty
                                            │        pulls ≥60 s post-end)
                                            v  (CAS)
                                       FINALIZED (terminal, immutable)

 boot recovery: rows in JOINING/LIVE/FINALIZING re-enter their state's loop
 (join retries to join_deadline_at / probe & tick / drain to grace_deadline_at);
 missing platform client ⇒ wait & retry, never abort (R4).
```

### 5.5 Restart recovery

Boot: repair every content log (4.2), then scan
`meetings WHERE status IN ('joining','live','finalizing')` and spawn
coordinators into their persisted state. No in-flight memory is required:
deadlines are absolute columns, the watermark is persisted with content,
segment state rebuilds from the log, and clients arrive via the startup
registry (5.1). A live-state probe that returns `MeetingEnded` transitions
through the normal end path (with whatever grace remains), not straight to
finalized.

## 6. Read protocol (CLI-mediated, cursor on the entity)

The load-bearing product idea: **increment logic lives on the read side.**
Agents are stateless; the entity remembers what each thread has confirmed.

### 6.1 CLI surface

`garyx meeting` subcommand (`commands/meeting.rs`, pattern B9):

- `garyx meeting list [--json]` — id, topic, status, closed-segment span,
  updated_at.
- `garyx meeting read <entity_id> [--full] [--range A..B] [--thread <id>] [--json]`
  — the only content access path. Cursor modes (`incremental` default,
  `--full`) require a thread identity: `GARYX_THREAD_ID` env, or `--thread`
  override; missing both is an error naming both remedies. `--range` is a
  stateless peek: it needs **no** thread identity, never touches cursors,
  and is the sanctioned anonymous-inspection path (R9 conflict resolved).
- `garyx meeting join <meeting_no> [--account <id>]` — debug trigger.

### 6.2 Gateway API

```
GET  /api/meetings                    -> list
GET  /api/meetings/{id}               -> metadata
POST /api/meetings/{id}/read          -> { thread_id?, mode, range?, receipt?, page_bytes? }
POST /api/meetings/{id}/join-debug    -> manual trigger
```

**Snapshot isolation (R8):** `/read` asks the entity's coordinator (live
entities) or reads directly (terminal entities) for a consistent snapshot
`(closed_latest, log_offset_of_closed_latest)`; slicing scans the log only
up to that offset. Appends beyond the snapshot are invisible; finalize/
delete take the same coordinator turn or fail the read with a clean
"entity deleted" error. Cursors only ever advance to segments actually
returned.

**Receipt-confirmed delivery (R7):** cursor state is
`confirmed_seq` + `pending(from,to)`.

1. Read (incremental): if a `pending` span exists, **re-serve it** (a prior
   response may not have arrived; at-least-once). Otherwise slice
   `(confirmed_seq, min(latest, page cap)]`, store it as `pending`, return
   it with `receipt = "<from>-<to>"`.
2. The CLI transparently sends the previous `receipt` with the *next* read
   (it appears in the returned header; agents re-running the command via
   the printed usage line carry it automatically because the CLI stores
   nothing — the receipt is echoed from the server's own pending state, so
   "next read confirms previous" needs no client state: a new read request
   for the same (entity, thread) confirms the pending span **iff** the
   request's `receipt` matches or the request explicitly asks to re-serve.
   Concretely: `read` with no flags = "confirm pending if any, then serve
   next increment"; `read --again` = "re-serve pending without confirming".
   A dropped response therefore re-serves the same span on the next attempt
   — duplicates possible, silent skips impossible.
3. `--full`: serves from segment 1 with pagination (below); each confirmed
   page advances `confirmed_seq` to that page's tail.
4. `--range A..B`: no cursor interaction at all.

Cursor writes are single transactions guarded by
`WHERE confirmed_seq = :expected` (CAS); concurrent reads from one thread
serialize on the row, and a stale writer loses harmlessly (cursors never
move backwards, R7).

**Pagination (R9):** responses are capped (default 64 KiB rendered / 200
segments, whichever first; `page_bytes` may lower it). A truncated response
says so in the header and the next `read` continues — this keeps every call
inside the CLI's 10 s HTTP budget regardless of meeting length. First read
of a long meeting is therefore a short loop of confirm-and-continue calls,
which the agent drives naturally because each header states "N segments
remain".

### 6.3 Self-describing output

```
── meeting entity 019f… ─ 《Q3 规划会》 ─ LIVE, updated 10:35:12 ──
Incremental read for this thread: segments 12–18 of 23 closed (5 remain)
receipt 12-18 pending — run `garyx meeting read 019f…` again to confirm & continue
To re-serve this span:   garyx meeting read 019f… --again
To read everything:      garyx meeting read 019f… --full   (paged)
To peek a span:          garyx meeting read 019f… --range 5..9   (no cursor)
────────────────────────────────────────────

[12] 10:32:05 张三 (transcript)
…
```

Rendered Markdown is generated from structured segments at response time;
segment headers are synthesized, so log content can never forge a segment
boundary (R6). Finalized/aborted entities state so with timestamps and
reason. Empty increments return the header plus "no new segments since
[confirmed 18]" — never an empty body.

### 6.4 Read/lifecycle edge tests live in Section 12.

## 7. Pointer injection and references

### 7.1 Resolver-before-dispatch (R11)

`gary_prompt` stays a synchronous formatter (B10). Meeting context is
resolved **before dispatch** where async and DB access already exist: a
`MeetingContextResolver` (gateway-implemented, injected into the dispatch
path alongside existing gateway-owned steps) runs for every user-turn entry
point — channel inbound, app/API sends, queued follow-ups, internal
dispatch. It point-reads `meeting_thread_refs` for the thread; for each ref
(bounded: newest 3 by `attached_at`, a documented cap) it point-reads entity
metadata + this thread's cursor and writes a compact
`meeting_context` value into run metadata. `gary_prompt` formats that value
into `<garyx_meeting_context>` next to `<garyx_thread_metadata>`:

```
<garyx_meeting_context>
entity_id: 019f…
topic: Q3 规划会
status: live (updated 2026-07-16 10:35:12)
closed_segments: 23
this_thread: confirmed 11, pending none (12 unread)
read: garyx meeting read 019f…
</garyx_meeting_context>
```

Resolver failure degrades to omitting the block (the CLI path still works);
it never blocks dispatch. No file paths anywhere.

### 7.2 Where refs come from

- **Entity card → "chat about this meeting"** (primary): creates/continues a
  thread and writes the `meeting_thread_refs` row in the same transaction as
  the thread-record write that anchors the action. From then on every turn
  in that thread carries the context block (contract item 6: recognition is
  the refs row + cursor row, pure server state).
- **CLI discovery** (secondary): `garyx meeting list` + `read` work from any
  thread without a ref; cursors behave identically. Reading does not create
  a ref; only the explicit user action does.

### 7.3 In-thread cards: deferred out of Phase 1 (R12)

Background insertion of `meeting_attached` control records would write a
foreign row into thread ledgers outside any run, exactly the
tail-reconciliation hazard the superseded design documented
(`store.rs:521`, B7). Phase 1 ships **no background ledger writes**: the
"chat about this meeting" turn itself carries the ref in its own user-turn
metadata (rendered as a chip from message metadata, no control record), and
entity state is always inspectable via the gallery and the context block.
A finalize-notification card, if wanted later, must go through the bridge's
per-thread run serialization with write-then-derive and stable-origin dedup
— a separate design.

## 8. UI surfaces (slice 3)

- **Desktop**: "Meetings" gallery in the left rail (live badge, topic,
  duration, closed-segment count). Refresh on open plus 30 s polling while
  visible — **no SSE in Phase 1** (B11/R13; a typed `/api/meetings/stream`
  with snapshot/reconnect semantics is future work if polling proves
  insufficient). Card action: "chat about this meeting" (7.2); delete for
  terminal entities.
- **iOS**: same catalog via gateway-scoped stale-while-refresh caching;
  route/mapping logic in `GaryxMobileCore` with SwiftPM tests; packaged
  `xcodebuild` validation per mobile rules.
- Chip rendering for ref-carrying turns comes from message metadata (7.3),
  not from new render-state control kinds.

## 9. Configuration and deployment prerequisites

- `FeishuAccount`: `#[serde(default = "default_true")] pub meeting_entities: bool`.
- `GatewayConfig`: `#[serde(default)] pub meetings: MeetingConfig`
  (`poll_interval_secs` default 30 clamp 10–120; `join_retry_window_secs`
  default 300; `read_page_bytes` default 65536).
- **Developer console (manual, blocks slice 2):** subscribe
  `vc.bot.meeting_invited_v1` + `vc.bot.meeting_ended_v1`, publish app
  version. Scopes already granted and live-verified.
- Public-repo hygiene: all fixtures sanitized/synthetic.

## 10. Failure and observability

- Lifecycle transitions log at info (entity id, feishu id, CAS from→to).
- Poll/join errors log with typed cause + backoff state; abort records
  `status_detail` visible in `garyx meeting list`.
- Boot repair logs truncated bytes / replayed lines / cursor rewinds.
- Metrics via logs in Phase 1.

## 11. Implementation impact map

| Area | Change |
|---|---|
| `garyx-models/src/local_paths.rs` | `default_meetings_dir()` |
| `garyx-models/src/config.rs` | `FeishuAccount.meeting_entities`, `GatewayConfig.meetings` |
| `garyx-channels/src/feishu/types.rs` | invite/ended event structs (fields pinned per 3.1) |
| `garyx-channels/src/feishu/ws.rs` | 2 dispatch branches → sink enqueue |
| `garyx-channels/src/feishu/client.rs` | `bots_join`/`bots_events` + `MeetingPlatformClient` adapter |
| `garyx-channels/src/meeting_sink.rs` (new) | `MeetingPlatformClient`, `MeetingEventSink`, `MeetingInvite`, typed errors, no-op impl |
| `garyx-channels/src/plugin.rs`, `feishu.rs` | sink/registry threading |
| `garyx-gateway/src/meetings/` (new) | service, per-entity coordinator, segment log writer/repair, read snapshotting, routes |
| `garyx-gateway/src/garyx_db/mod.rs` | 3 tables + indexes + CRUD + thread-delete/archive cleanup hooks |
| `garyx-gateway/src/route_graph.rs` | 4 routes |
| `garyx-gateway/src/composition/*` | service wiring, sink injection, resolver injection |
| dispatch path (gateway/router seam) | `MeetingContextResolver` before dispatch |
| `garyx-bridge/src/gary_prompt.rs` | format `meeting_context` metadata into the block (sync) |
| `garyx/src/commands/meeting.rs`, `cli.rs`, `main.rs` | CLI |
| Desktop / iOS | gallery + ref chip (slice 3) |

## 12. Test plan (headless-first; expanded per R15)

**Lifecycle/coordinator (deterministic, mock platform client):**
simultaneous push+pull end; EndedSignal during an in-flight poll page; end
during joining; duplicate invites with distinct event_ids (unique-index
path); WS redelivery of one event_id; restart during joining / finalizing /
mid-pagination; 10005-during-live → aborted; grace drain quiescence vs
deadline; finalized/aborted both refuse appends; missing client at recovery
→ wait, not abort.

**Storage fault injection:** crash between log flush and SQLite commit
(cursor behind → idempotent re-pull via `sources` dedup); torn final JSONL
line truncation; crash between row-create and dir-create; delete
tombstone sweep; boot re-count matches checkpoint.

**Watermark/pagination:** new events mid-pagination; last-page token
persistence; duplicate page replay dedup; resume-after-restart from
persisted token.

**Read protocol:** first read paged; confirm-and-continue loop; `--again`
re-serve; response committed but connection dropped → same span re-served
(no silent skip); out-of-order cursor CAS (stale writer loses; cursor never
regresses); read-vs-append snapshot; read-vs-finalize; read-vs-delete;
`--range` without thread identity; incremental/full without identity →
error; cursor rows die with thread delete and cannot resurrect; per-thread
isolation.

**CLI:** `GARYX_THREAD_ID` resolution/override/missing; golden header tests
(human + `--json`), including pending-receipt and truncation notices.

**Resolver/prompt:** refs present/absent/capped; cursor point-read failure
degrades to no block; every dispatch entry path gets the resolver (channel,
app, queued follow-up, internal).

**Cross-crate:** gateway implements the channels traits (real compile);
no-op sink assembly; registry re-population on channel restart.

**Rust scope:** `cargo test -p garyx-channels -p garyx-gateway
--all-targets` plus tier1 fast loop; slice 3 adds `GaryxMobileCore` SwiftPM,
`xcodebuild`, and a packaged desktop check per repo validation rules.

## 13. Delivery slices

1. **Entity core + read protocol** (no Feishu dependency): tables, segment
   log + repair, snapshot reads, receipt cursors, `meeting list/read`,
   resolver + context block, fixture tests. Gate: full read-protocol and
   storage-fault suites green; hand-seeded entity readable incrementally
   from two threads with independent receipt-confirmed cursors.
2. **Ingestion**: sample capture + fixture pinning (3.1) first; event
   structs, WS branches, sink + registry, coordinator with CAS lifecycle,
   poll watermark, grace drain, recovery, `meeting join` debug command,
   console subscriptions. Gate: real invited meeting → live→finalizing→
   finalized entity with correct segments; restart mid-meeting and
   mid-finalizing resume correctly; lifecycle suite green.
3. **Experience**: galleries, ref chips, polling refresh. Gate: card → chat
   → agent reads increment → answer cites new segments, end to end on
   packaged desktop + simulator.

## 14. Open questions (defaults chosen, non-blocking)

- Retention: manual deletion only.
- One entity per (account, meeting occurrence); cross-account dedup out of
  scope.
- `meeting_activity_v1` ignored in Phase 1 (poll captures joins/leaves).
