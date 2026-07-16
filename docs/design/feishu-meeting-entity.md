# Feishu meeting entity

Status: revision 6 — self-contained specification (no external revision
references), addressing adversarial review #TASK-2337 rounds 1–5.
Author: gary (design)
Scope ruling (user, 2026-07-16): the meeting entity is an **orthogonal
side-system**. Existing runtime flows are not modified. Agents read it via
CLI; conversation references are plain text; read cursors live on the read
side.
Supersedes: `docs/design/feishu-group-listen-mode.md` (group listening
cancelled).

## 1. Summary

When someone invites the Garyx bot into an ongoing Feishu meeting, the bot
joins, polls in-meeting events, and materializes a **meeting entity** in a
self-contained subsystem: its own SQLite tables, its own on-disk segment
log, its own gateway routes, its own CLI. Capture is durably checkpointed
at bounded tick granularity. When the meeting ends, capture drains the
platform grace window and the entity becomes immutable.

Agents read entities exclusively through `garyx meeting read`. The server
keeps a per-(entity, reader) cursor; a reader's first incremental read
returns content from the start and *is* the recognition of a prior
reference; later reads return increments via a linearized fetch/confirm
protocol — at-least-once, never silently skipped while the platform data
is still on disk.

## 2. Product contract (user-approved 2026-07-16; scope reset same day)

1. Meetings only. No group-chat listening.
2. Start: an in-meeting invite admits, joins, polls. `garyx meeting join`
   is a debug path.
3. Live: near-real-time accumulation (~30 s platform transcript latency).
4. Read protocol: CLI-only content access; per-reader server cursors;
   incremental by default; self-describing output.
5. End: meeting-ended signal stops capture; entity freezes.
6. Reference continuity: the cursor row is the recognition of a prior
   reference — no other machinery.
7. **Orthogonality:** no existing flow changes. No prompt injection, no
   bridge/provider/dispatch instrumentation, no render-state changes, no
   thread-lifecycle hooks. Additive touchpoints only (Section 11).

Product sign-off items (accepted limits):

- **S1** Feishu WS ACKs before processing; an invite is lost iff the
  process dies or the admission insert exhausts 3 bounded retries in the
  ACK→admission window (error-logged).
- **S2** One entity per admission: re-invites after terminal/deletion
  create new entities.
- **S3** The final ~30 s of speech becomes readable during the grace
  drain, not instantly.
- **S4** A crash normally loses nothing (unfinished ticks are re-pulled).
  If the platform becomes unreadable before the re-pull — grace window
  expired while the process was down, or the bot was removed — at most
  one bounded tick (≤ 1 MiB text / 10 s of capture) is **permanently and
  silently lost**. Accepted.
- **S5** Agents learn about entities from referenced text, from
  `garyx meeting list`, or from memory. No automatic per-turn injection;
  an unreferenced entity may go unnoticed by an agent. Accepted.
- **S6** Access model: Garyx is a single-principal system. **Every
  gateway-authenticated caller has read access to the full meeting
  catalog and all entity content.** The reader id used by cursors is a
  cooperative bookmark key, not an authorization boundary; any caller
  presenting the same reader id shares that cursor. Accepted.

## 3. Verified repository baseline

| # | Fact | Evidence |
|---|---|---|
| B1 | Feishu WS: the protobuf data-event path ACKs before processing; a raw-text fallback path exists without the same ACK evidence. Only `im.message.receive_v1` is handled today; 30 min event-id dedup cache | `garyx-channels/src/feishu/ws.rs:968,997,1040,1049-1072`, `feishu.rs:95` |
| B2 | `FeishuChannel::new(config, router, bridge, dispatcher, public_url)` → `FeishuRuntimeContext`; constructor injection precedent | `feishu.rs:370`, `ws.rs:38-69`, `plugin.rs:3235,3322` |
| B3 | gateway depends on channels; a channels-defined, gateway-implemented trait compiles. First such **production service seam** (gateway tests already implement channels traits) | `garyx-gateway/Cargo.toml:20` |
| B4 | `FeishuClient` is `Clone` but `pub(crate)` | `client.rs:127` |
| B5 | `CronService` background-service shape | `cron.rs:633,713-796,799-851` |
| B6 | Capsule storage precedent (disk + STRICT SQLite + migration) | `capsules.rs:158,273`, `garyx_db/mod.rs:2645,3559` |
| B7 | `GARYX_THREAD_ID` is exported to agent processes launched with runtime metadata (the normal agent path); not guaranteed for arbitrary direct bridge invocations. CLI precedent reads it | `gary_prompt.rs:148-154`, `commands/task.rs:233` |
| B8 | CLI→gateway pattern; 10 s mutation timeout → paging budget | `gateway_client.rs:15,73,101,220` |

External platform facts: `POST /open-apis/vc/v1/bots/join` (9-digit
`meeting_no` + optional `call_id` → long `meeting.id`; tenant token);
`GET /open-apis/vc/v1/bots/events` (pull, `page_token` continuation —
opaque: stored and resumed from, never compared); event types
`participant_joined/left`, `chat_received`, `transcript_received`,
`magic_share_started/ended`; ~30 s transcript latency (5 s/100-item
batches); multi-party only; per-meeting owner switch; `10005` bot not in
meeting; 5-minute post-end grace window; `20001` window over. Console must
subscribe `vc.bot.meeting_invited_v1` / `vc.bot.meeting_ended_v1`.

Typed platform errors:
`MeetingApiError = NotInMeeting | GraceExpired | AuthFailed |
RetriableTransport | Other(code, msg)`. `AuthFailed` is produced by a
fixed mapping table from Feishu auth-class codes (populated at
implementation from the official error table; the design requires the
mapping to exist and default unknown codes to `Other`).

### 3.1 Sample-pinning gate

Slice 2 opens by capturing sanitized fixtures: invite/ended envelopes,
join response, events pages (including grace-window reads and the `20001`
response). Pinned facts: invite payload fields; joining-stage identity;
existence or absence of an in-band ended item (if absent, end sources are
exactly `push` and `grace_expired`); grace-window read behavior. Mismatch
stops slice 2 for a design amendment. Slice 1 has no Feishu dependency.

## 4. Entity model and storage

### 4.1 SQLite DDL (normative)

```sql
CREATE TABLE meetings (
  id                   TEXT NOT NULL PRIMARY KEY,      -- uuid_v7
  account_id           TEXT NOT NULL,
  meeting_no           TEXT NOT NULL,
  feishu_meeting_id    TEXT NOT NULL DEFAULT '',
  invite_event_id      TEXT NOT NULL,
  call_id              TEXT NOT NULL DEFAULT '',
  topic                TEXT NOT NULL DEFAULT '',
  invited_by           TEXT NOT NULL DEFAULT '',
  status               TEXT NOT NULL CHECK (status IN
    ('joining','live','finalizing','aborting','finalized','aborted')),
  status_detail        TEXT NOT NULL DEFAULT '',
  stalled_reason       TEXT NOT NULL DEFAULT '' CHECK (stalled_reason IN
    ('','no_client','auth_failed','transport')),
  failure_since        TEXT,                            -- persisted stall clock
  end_source           TEXT NOT NULL DEFAULT '' CHECK (end_source IN
    ('','push','poll_ended','grace_expired')),
  join_deadline_at     TEXT NOT NULL,
  grace_deadline_at    TEXT,
  poll_cursor          TEXT NOT NULL DEFAULT '',        -- cache; log ckpt chain is truth
  closed_segment_count INTEGER NOT NULL DEFAULT 0 CHECK (closed_segment_count >= 0),
  byte_size            INTEGER NOT NULL DEFAULT 0 CHECK (byte_size >= 0),
  started_at           TEXT NOT NULL,
  ended_at             TEXT,
  finalized_at         TEXT,
  created_at           TEXT NOT NULL,
  updated_at           TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX idx_meetings_invite ON meetings(invite_event_id);
CREATE UNIQUE INDEX idx_meetings_active_no
  ON meetings(account_id, meeting_no)
  WHERE status IN ('joining','live','finalizing','aborting');
CREATE UNIQUE INDEX idx_meetings_active_fid
  ON meetings(account_id, feishu_meeting_id)
  WHERE feishu_meeting_id <> ''
    AND status IN ('joining','live','finalizing','aborting');
CREATE INDEX idx_meetings_created ON meetings(created_at DESC, id);
CREATE INDEX idx_meetings_status  ON meetings(status);

CREATE TABLE meeting_read_cursors (
  meeting_id    TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  reader_id     TEXT NOT NULL CHECK (length(reader_id) BETWEEN 1 AND 128),
  confirmed_seq INTEGER NOT NULL DEFAULT 0 CHECK (confirmed_seq >= 0),
  pending_from  INTEGER,
  pending_to    INTEGER,
  receipt       TEXT,
  updated_at    TEXT NOT NULL,
  PRIMARY KEY (meeting_id, reader_id),
  CHECK ((pending_from IS NULL) = (pending_to IS NULL)
     AND (pending_from IS NULL) = (receipt IS NULL)
     AND (pending_from IS NULL OR
          (pending_from > confirmed_seq AND pending_to >= pending_from))
) STRICT;
```

All timestamps are UTC RFC3339 with `Z`. `reader_id` is the trimmed,
non-empty value of `GARYX_THREAD_ID` or `--thread` (S6: cooperative key,
no authorization semantics; the subsystem never validates it against
`thread_records`, hooks no thread lifecycle, and inert rows for dead
threads are cleaned by entity-deletion cascade). There is no refs table.

### 4.2 Content log (normative)

`~/.garyx/meetings/{entity_id}/segments.jsonl` — two line kinds:

```
{"t":"seg","seq":12,"kind":"transcript","speaker":"张三","start":"…","end":"…",
 "text":"…","sources":["sent_8813","sent_8814"],"cont":false}
{"t":"ckpt","cursor_out":"pt_x9y8…","at":"2026-07-16T02:35:12Z"}
```

- `seq`: dense integers assigned at segment close; segments close at
  speaker change, 60 s gap, non-transcript event, or tick close.
  Coalescing never crosses a tick close.
- **Write-time size splitting:** before append, a segment whose *final
  encoded line* would exceed 32 KiB (measured on the rendered JSON bytes,
  after escaping — escape inflation counted) is split at UTF-8 character
  boundaries into consecutive independent seqs, `"cont":true` on
  continuations.
- `sources`: platform item ids, used for cross-page/page-replay dedup
  during a tick. They are provenance, **not** a recovery dedup key
  (see repair).
- **Durability boundary = the `ckpt` line.** The tick-close sequence
  (under the entity I/O write lock): close coalescing → append all `seg`
  lines → append one `ckpt` line (every tick, including empty ones) →
  `fdatasync` → SQLite cache transaction (`poll_cursor`,
  `closed_segment_count`, `byte_size`, `updated_at`). Nothing before the
  `ckpt` is considered captured.
- **Boot repair (R5-01):** for every **non-terminal** entity (terminal
  logs are never scanned at boot): validate line by line; find the last
  valid `ckpt`; **truncate everything after it** — torn lines *and*
  complete `seg` lines alike (they belong to an unfinished tick). Resume
  the poll from that `ckpt`'s `cursor_out` and re-pull the whole tick.
  This makes recovery atomic at tick granularity and immune to the
  split-segment/source-dedup interaction: partially persisted splits are
  discarded wholesale, then re-materialized. Rebuild the SQLite caches
  and the in-memory offset index in the same pass.
- **Loss window (S4):** content that existed only in an unfinished tick
  is recovered by the re-pull while the platform still serves it; if the
  grace window expired or the bot lost access while the process was
  down, that single bounded tick is permanently lost (signed off).

Markdown is a render format; no on-disk Markdown, headers, or end
markers.

### 4.3 Entity deletion

Terminal entities only. Under the entity I/O write lock: atomic
`rename {id}/ → {id}.tombstone/` (a missing dir is a legal empty entity —
skip the rename) → DB `DELETE` (cascades cursors) → remove tombstone.
Boot order: **tombstone reconcile before log repair** — tombstone+row →
rename back; tombstone w/o row → remove; bare dir w/o row → remove
(logged). `DELETE /api/meetings/{id}` (409 non-terminal); `garyx meeting
delete`. Confirm (6.4) against a deleted entity returns 404 and the CLI
reports "entity deleted" — no cursor promise survives deletion.

## 5. Ingestion

### 5.1 Platform seam (channels crate)

New `garyx-channels/src/meeting_sink.rs`:

```rust
#[async_trait]
pub trait MeetingPlatformClient: Send + Sync {
    async fn join(&self, meeting_no: &str, call_id: Option<&str>)
        -> Result<JoinedMeeting, MeetingApiError>;
    async fn poll_events(&self, feishu_meeting_id: &str, page_token: &str)
        -> Result<EventsPage, MeetingApiError>;   // one page per call
}
pub trait MeetingEventSink: Send + Sync {
    fn register_client(&self, account_id: &str, client: Arc<dyn MeetingPlatformClient>);
    fn unregister_client(&self, account_id: &str);
    fn on_meeting_invited(&self, invite: MeetingInvite);
    fn on_meeting_ended(&self, account_id: &str, feishu_meeting_id: &str);
}
```

- The adapter over `FeishuClient` lives in garyx-channels (B4);
  `FeishuChannel::start`/`stop` register/unregister per enabled account;
  re-register replaces. `poll_events` returns **one page per call** so
  the coordinator owns deadlines between pages (R5-04).
- Channel-code changes: two new `else if` event branches in `ws.rs`, two
  event structs in `types.rs`, two REST methods + adapter in `client.rs`,
  one constructor dependency threaded through `plugin.rs`/`feishu.rs` —
  all additive (the full file list is Section 11; no existing behavior
  changes).
- `on_meeting_invited` = admission insert (3 bounded retries with
  100 ms/1 s backoff, error-logged, S1) + coordinator nudge. Nothing
  else runs on the WS loop.

### 5.2 Coordinator (gateway crate)

`MeetingService` (CronService shape; `Arc` on AppState, stop channel,
`Weak<AppState>`): one coordinator task per non-terminal entity consuming
a command queue (`Nudge`, `EndedSignal(source)`, `AbortRequest`,
`Shutdown`, self-scheduled `PollTick`).

**Command scheduling (R5-04):** commands are processed **immediately
whenever no tick is actively executing** — a joining entity (no ticks) or
a stalled live entity handles admin aborts at once. A command arriving
during an executing tick is processed at that tick's close (bounded ≤
budget + one uncancellable durability flush).

**Durable CAS transitions:** every state change is
`UPDATE meetings SET status=:to … WHERE id=:id AND status=:from`; losers
are no-ops. Intent stages make terminals crash-resumable:

- finalize: `live → finalizing` (drain runs here) → `finalizing →
  finalized` (index persistence happens inside finalizing; 6.3).
- abort: `joining|live → aborting` (record `status_detail`) → flush
  (final `ckpt` if a tick was interrupted) → `aborting → aborted`.
- Terminal-affecting inputs (`EndedSignal`, `AbortRequest`,
  `NotInMeeting`) queued together resolve with priority **end > abort**.
- `finalized` and `aborted` both refuse appends (checked under the I/O
  lock).

**Admission → joining:** the sink insert creates `status='joining'`,
`join_deadline_at = now + join_retry_window` (absolute). Unique indexes
make duplicate invites no-ops (S2). Content dir is created lazily by the
writer (`ensure_dir` before first append). Join retries every 20 s until
success (CAS `joining→live`, backfill long id + topic) or the deadline
(abort path). The deadline applies **even while stalled** — it models
invite validity. Restart resumes retrying to the same absolute deadline.
Join succeeding during the grace window is legal; polls then return data
until `GraceExpired` finalizes.

**Live ticks (R5-04):** a tick has a 10 s **fetch/accumulation budget**:
each `poll_events` page call runs under the tick's remaining deadline
(`tokio::time::timeout`); on timeout the tick closes with the pages that
completed (an in-flight page is dropped; its token is not consumed —
`cursor_out` is the last *completed* page's token). A tick also closes at
`has_more=false`, 10 pages, 1000 items, or 1 MiB of accumulated text.
A single oversized page closes the tick by itself: its items are
committed (split as needed, 4.2) with `cursor_out` = that page's token;
if one page's items alone exceed bounds they are committed across
**multiple consecutive ticks that reuse the same `cursor_out`** (re-pull
safe: `sources` dedup absorbs the overlap within the resumed tick). After
a bounds-forced close with more data available, the next tick is
scheduled immediately; otherwise on the normal 30 s cadence (jittered).

Transport errors back off 30→60→120 s within the schedule. `NotInMeeting`
while live (no end signal) → abort path. `GraceExpired` while live →
finalize immediately (`end_source='grace_expired'`). `AuthFailed` /
`RetriableTransport` persisting: `failure_since` is set on the first
consecutive failure (persisted; survives restart), cleared on any
success; when `now - failure_since > 15 min` the entity shows
`stalled_reason` (`auth_failed`/`transport`); `no_client` shows whenever
the registry lacks the account's client. Stalled live entities are never
auto-aborted; `garyx meeting abort` (joining|live only) is the manual
exit.

### 5.3 End and grace drain

`EndedSignal(push)` (or `poll_ended` if pinned by 3.1): CAS
`live→finalizing`, `ended_at=now`, `grace_deadline_at = now + 4 min`.
Drain: keep ticking on the normal cadence **until the deadline** — no
quiescence shortcut. Then final `ckpt`, persist the offset index (6.3),
CAS `finalized`. `GraceExpired` during finalizing accelerates completion;
`NotInMeeting` during finalizing completes early; `AbortRequest` during
finalizing is refused; a stalled finalizing entity still finalizes at the
deadline. Unknown-entity end signals: logged, dropped (grace-window polls
are the correlation-free backstop). Restart during finalizing resumes the
drain to the persisted deadline.

### 5.4 State machine

```
 invite ─admission insert─> JOINING ─join ok─> LIVE ─bounded ticks─┐
   │      (unique keys, S1/S2)  │(absolute         │               │ commands: immediate
   │                            │ deadline,        │ Ended/Grace   │ when idle; at tick
   │                            │ even stalled)    │               │ close when executing
   │                            v                  v               │ (end > abort)
   │                        ABORTING ─flush─> ABORTED         FINALIZING ─drain to deadline;
   │                            ^                              │ GraceExpired/10005 ⇒ early
   │                            └──10005 while live────────────│ complete; abort refused;
   │                                                           │ stalled ⇒ finalize at deadline;
   │                                                           │ index persisted here
   │                                                           v
   │                                                      FINALIZED
 boot: tombstone reconcile → log repair (non-terminal only: truncate past
 last ckpt, rebuild caches+index) → coordinators resume persisted stages.
```

## 6. Read protocol

### 6.1 Surfaces

CLI: `garyx meeting list [--json]`;
`garyx meeting read <id> [--full | --range A..B] [--thread <id>] [--json]
[--max-bytes N]`; `garyx meeting join <no>`; `garyx meeting abort <id>`;
`garyx meeting delete <id>`.

API: `GET /api/meetings` (paged), `GET /api/meetings/{id}`,
`POST /api/meetings/{id}/read`, `POST /api/meetings/{id}/read/confirm`,
`POST /api/meetings/{id}/abort`, `DELETE /api/meetings/{id}`,
`POST /api/meetings/{id}/join-debug`.

Incremental mode (default) requires a reader identity (`GARYX_THREAD_ID`
env or `--thread`; missing both → error naming both remedies).
`--full`/`--range` are stateless paged snapshot peeks: no identity, no
cursor rows ever created or touched. Only a successful incremental fetch
creates a cursor row (R5-08).

### 6.2 List pagination (R5-07)

Ordered by the immutable key `(created_at DESC, id DESC)` with keyset
continuation `{created_at, id}` — rows never move under this order, so
pagination is stable by construction under concurrent updates, inserts
(new rows sort before any already-served page and are absent from an
in-progress traversal — documented, acceptable for a gallery), and
deletions. No snapshot machinery. Live-first presentation is a client
sort of the fetched page set, not a server order.

### 6.3 Locking, snapshots, offset index (R5-06)

- Per-entity `RwLock` (service map) over all states: appends/finalize
  flush/delete take write; reads take read and capture
  `(closed_latest, log_offset_of_closed_latest)`; slicing never scans
  past the snapshot offset.
- Sparse offset index (every 64th seq → byte offset). Live: built during
  boot repair, maintained per append, memory-only. Terminal:
  `{id}/index.bin` written **inside finalizing/aborting, before the
  terminal CAS**, as temp-file → `fdatasync` → atomic rename; header =
  `{version, log_byte_len, latest_seq, crc32}`. On read, a missing,
  torn, version-mismatched, or `log_byte_len`/`crc`-mismatched index is
  discarded and rebuilt once (synchronously; the rebuild re-persists via
  the same atomic sequence). A first read racing a slow rebuild may
  exceed the CLI budget and time out; the retry hits the persisted
  index. Crash between terminal CAS and a failed index write is
  impossible by ordering (index precedes CAS); crash before the CAS
  re-enters the intent stage which rewrites the index idempotently.
- Response page cap: 64 KiB rendered / 200 segments (min with
  `--max-bytes`). Write-time splitting (4.2) guarantees no single
  segment line exceeds 32 KiB, so every segment fits a page.

### 6.4 Incremental fetch/confirm (linearized, R5-03)

Cursor row algebra (single SQLite transactions):

1. **Ensure row:** `INSERT INTO meeting_read_cursors (meeting_id,
   reader_id, confirmed_seq, updated_at) VALUES (…, 0, now)
   ON CONFLICT DO NOTHING`. (Created on first incremental fetch even if
   the increment is empty — the row is the recognition state.)
2. **Fetch:** read the row. If `pending IS NOT NULL` → re-serve exactly
   `(pending_from..pending_to)` with the stored receipt. Else compute the
   next span `(confirmed_seq, min(latest_snapshot, confirmed_seq +
   page))` and claim it:
   `UPDATE … SET pending_from=:f, pending_to=:t, receipt=:r,
   updated_at=now WHERE meeting_id=:m AND reader_id=:rd AND
   confirmed_seq=:expected AND pending_from IS NULL`.
   Zero rows updated → a concurrent fetch won → re-read and re-serve the
   winner's pending span and receipt (both callers deliver identical
   content; receipts are shared, not per-caller).
   An empty span (`confirmed_seq == latest`) serves the empty-increment
   header without claiming pending.
3. **Confirm** (`/read/confirm {receipt}`):
   `UPDATE … SET confirmed_seq=pending_to, pending_from=NULL,
   pending_to=NULL, receipt=NULL, updated_at=now
   WHERE meeting_id=:m AND reader_id=:rd AND receipt=:receipt`.
   Unknown/stale receipt → no-op success (idempotent). Deleted entity →
   404 (cursor rows died with it).
4. The CLI performs fetch → render → stdout flush → confirm in one
   invocation. Crash before flush ⇒ pending survives ⇒ same span
   re-serves. Confirm committed but response lost ⇒ next read serves the
   next span (safe: content already flushed). Cursors never regress.

### 6.5 Stateless snapshots (`--full`, `--range`)

The first response pins `(closed_latest, log_offset)` and returns an
opaque continuation token `{entity_id, snapshot, next_seq, mode,
range_end, checksum, expiry 10 min}`; the CLI loops within one
invocation streaming pages to stdout until exhausted, or stops at
`--max-bytes` and prints the resume command with the token. Appends
beyond the snapshot are invisible to that token's pages.

### 6.6 Self-describing output

Every response names: mode; exact span served; totals; entity status
(live / finalized / aborted with `end_source`, `stalled_reason`,
timestamps); for incremental — confirmation state and "re-read any span:
`--range`"; for empty increments — "no new segments since [N]"; for
first reads — "first read for this reader". Rendered Markdown is
synthesized from structured segments; log text cannot forge headers.

## 7. Conversation references: a text convention

A reference is a line of text containing the **entity id and the fixed
read command only — no topic or other platform text** (platform strings
are untrusted and could carry newlines/Bidi/prompt-injection; R5-09):

```
[meeting 019f7abc-… — read: garyx meeting read 019f7abc-…]
```

- UI "chat about this meeting" (slice 3) prefills the composer with that
  line; the user may edit; the message travels existing paths unchanged.
  The agent sees the id and runs the CLI (which prints the topic and
  status as trusted, server-rendered output).
- Recognition of a prior reference is the cursor row (6.4 step 1).
- Discovery without a reference: `garyx meeting list`.
- A deleted entity makes the command fail "entity not found"; stale
  reference lines are harmless history.

## 8. UI surfaces (slice 3)

Desktop "Meetings" gallery (left rail): keyset-paged list
(`created_at` order, client-side live-first presentation), refresh on
open + 30 s polling while visible (no SSE); actions: chat-about
(prefill), abort (joining|live), delete (terminal). iOS: same catalog via
stale-while-refresh; logic in `GaryxMobileCore` + SwiftPM tests; packaged
`xcodebuild` validation. No transcript/render_state work.

## 9. Configuration and deployment prerequisites

`FeishuAccount.meeting_entities: bool` (`default_true`; disabling stops
new admissions and unregisters the client; existing entities follow 5.2
stalled rules). `GatewayConfig.meetings: MeetingConfig`
(`poll_interval_secs` 30 clamp 10–120; `join_retry_window_secs` 300;
`read_page_bytes` 65536). Console: subscribe the two `vc.bot.*` events,
publish app version (scopes already granted and live-verified). Fixtures
sanitized.

## 10. Failure and observability

Lifecycle transitions log info (entity, feishu id, CAS from→to, source);
typed errors with backoff and `failure_since` state; `status_detail`,
`end_source`, `stalled_reason` in `garyx meeting list --json`; boot
repair logs truncated bytes/lines (including whole-seg truncation),
cursor corrections, tombstone dispositions, index rebuilds.

## 11. Implementation impact map

| Area | Change | Nature |
|---|---|---|
| `garyx-models/src/local_paths.rs` | `default_meetings_dir()` | additive |
| `garyx-models/src/config.rs` | 2 serde-default fields | additive |
| `garyx-channels/src/meeting_sink.rs` (new) | traits/types/no-op | additive |
| `garyx-channels/src/feishu/types.rs` | 2 event structs | additive |
| `garyx-channels/src/feishu/ws.rs` | 2 dispatch branches | additive |
| `garyx-channels/src/feishu/client.rs` | 2 REST methods + adapter | additive |
| `garyx-channels/src/plugin.rs`, `feishu.rs` | 1 constructor dep | additive |
| `garyx-gateway/src/meetings/` (new) | service/coordinator/log/locks/index/routes | new module |
| `garyx-gateway/src/garyx_db/mod.rs` | 2 tables + CRUD | additive |
| `garyx-gateway/src/route_graph.rs` | 7 routes | additive |
| `garyx-gateway/src/composition/*` | wiring + sink injection | additive |
| `garyx/src/commands/meeting.rs`, `cli.rs`, `main.rs` | CLI | additive |
| Desktop / iOS (slice 3) | gallery + prefill | additive |

**Not touched:** bridge, providers, router dispatch, transcripts,
render_state, SSE, thread lifecycle, prompt assembly, queued-input
paths.

## 12. Test plan

**Storage/repair:** ckpt-boundary truncation drops whole unfinished-tick
`seg` lines (incl. partial split families) and re-pull re-materializes
them; torn-line truncation; escape-inflation splitting (raw < 32 KiB,
encoded > 32 KiB); crash after each split chunk with no ckpt; empty-tick
ckpt; `fdatasync` ordering fault injection; cache rebuild equals log.

**Ticks/scheduling:** hanging `poll_events` cut by remaining-deadline
timeout (in-flight page token unconsumed); oversized single page commits
across consecutive same-cursor ticks with dedup; sustained `has_more`
closes at every bound with immediate reschedule; command executed
immediately when idle (joining + stalled-live admin abort) vs at close
when executing.

**Lifecycle:** end>abort arbitration; GraceExpired live/finalizing;
10005 live/finalizing; stalled matrix (`no_client`/`auth_failed`/
`transport`, `failure_since` persisted across restart — 14 min before +
2 min after; cleared on success); joining deadline while stalled;
finalizing finalizes at deadline while stalled; admission-insert failure
after retries (S1); reinvite after terminal and after delete (S2);
restarts in every non-terminal state; **S4 loss-window cases**:
finalizing crash near deadline + recovery after expiry (bounded loss,
no corruption), live crash + `10005` on recovery.

**Deletion:** all crash points of rename→delete→remove; missing-dir
empty entity; tombstone-before-repair ordering; confirm/read against
deleted entity (404).

**Fetch/confirm:** concurrent first fetch (single row, shared receipt);
concurrent fetch on existing row with empty pending (one winner, loser
re-serves winner's span); fetch/confirm/delete tripartite race;
crash-before-flush vs confirm-committed-response-lost (distinct);
receipt CAS stale no-op; cursors never regress; empty increment creates
the row; `--full`/`--range` never create rows; reader-id trim/empty/
overlength rejection; same reader id from two "threads" shares a cursor
(S6 documented behavior).

**Snapshots/index:** `--full` across pages pinned to one snapshot under
concurrent appends; token expiry/checksum; index written before terminal
CAS (crash between → intent stage rewrites); torn/stale/version-bumped/
crc-mismatched index discarded and rebuilt-once; cold high-seq read on a
long terminal log within budget via persisted index; rebuild-timeout
retry path.

**List:** keyset stability while rows update (the R5-07 counterexample:
an unseen row updated mid-traversal is still returned), insert/delete
during traversal.

**Access (S6):** any authenticated caller lists/reads all accounts'
entities; arbitrary reader ids create bounded rows (length cap).

Scope: `cargo test --all-targets` for `garyx-channels`, `garyx-gateway`,
`garyx-models`, `garyx` (CLI); tier1 fast loop; slice 3 adds SwiftPM +
`xcodebuild` + packaged desktop check.

## 13. Delivery slices

1. **Entity core + read protocol** (no Feishu): DDL, log writer/repair,
   locks + index, fetch/confirm + snapshots, CLI list/read/delete,
   fixture tests. Gate: read-protocol, storage-fault, deletion, and
   concurrency suites green; hand-seeded entity readable incrementally
   from two reader ids with independent receipt-confirmed cursors
   including forced-failure re-serves.
2. **Ingestion**: sample capture gate (3.1) → events/sink/registry,
   coordinator + intent CAS, bounded ticks, grace drain, recovery, admin
   abort, console subscriptions. Gate: real meeting live→finalizing→
   finalized with correct segments; restarts in every non-terminal state
   resume; lifecycle + stalled suites green.
3. **Experience**: galleries + prefill chat + polling. Gate: card →
   prefilled message → agent reads increment → answer cites new
   segments, on packaged desktop + simulator.

## 14. Open questions (defaults chosen, non-blocking)

- Retention: manual deletion only.
- One entity per admission (S2).
- `meeting_activity_v1` ignored in Phase 1.
- Per-turn automatic context injection: explicitly out of scope; a
  future separate design if ever wanted.
