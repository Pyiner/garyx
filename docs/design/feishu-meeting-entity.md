# Feishu meeting entity

Status: revision 7 — self-contained specification, addressing adversarial
review #TASK-2337 rounds 1–6.
Author: gary (design)
Scope ruling (user, 2026-07-16): orthogonal side-system; existing runtime
flows are not modified; agents read via CLI; conversation references are
plain text; read cursors live on the read side.
Supersedes: `docs/design/feishu-group-listen-mode.md`.

## 1. Summary

When someone invites the Garyx bot into an ongoing Feishu meeting, the bot
joins, polls in-meeting events, and materializes a **meeting entity** in a
self-contained subsystem: its own SQLite tables, on-disk segment log,
gateway routes, and CLI. Capture is durably committed **one platform page
at a time** — each poll fetches exactly one page and commits it atomically
with its checkpoint, so there is no multi-page in-memory state to lose and
no cursor ambiguity to reconcile. When the meeting ends, capture drains
the platform grace window and the entity becomes immutable.

Agents read entities exclusively through `garyx meeting read`: per-reader
server cursors, linearized fetch/confirm, at-least-once delivery, output
that always states what it is. All platform-derived text is handled as
untrusted data end to end.

## 2. Product contract (user-approved 2026-07-16; scope reset same day)

1. Meetings only. 2. Invite-driven join (+ debug `garyx meeting join`).
3. Live near-real-time accumulation (~30 s platform latency).
4. CLI-only reads, per-reader cursors, incremental default,
   self-describing. 5. End signal freezes the entity. 6. The cursor row is
   the recognition of prior reference. 7. Orthogonality: additive
   touchpoints only (Section 11); no existing flow changes.

Product sign-off items (accepted limits — each is an explicit product
decision, revocable by the owner):

- **S1** On the Feishu WS **protobuf data-event path**, events are ACKed
  before processing; an invite on that path is lost iff the process dies
  or the admission insert exhausts 3 bounded retries in the ACK→admission
  window (error-logged). The raw-text fallback path has no equivalent ACK
  evidence; the sample gate (3.1) pins which path carries
  invite/ended events, and if the raw-text path can carry them its
  delivery semantics get a design amendment before slice 2 proceeds.
- **S2** One entity per admission: re-invites after terminal/deletion
  create new entities.
- **S3** The final ~30 s of speech becomes readable during the grace
  drain.
- **S4** A crash normally loses nothing (an uncommitted page is
  re-pulled). If the platform becomes unreadable before the re-pull
  (grace expired while down; bot removed), at most **one platform events
  page** (page_size ≤ 100 items) is permanently and silently lost.
- **S5** Agents learn about entities from referenced text,
  `garyx meeting list`, or memory; no automatic per-turn injection.
- **S6** Access/trust model: Garyx is a single-principal personal system.
  Every gateway-authenticated caller has read access to the full meeting
  catalog and all content. Because agents act on behalf of anyone who can
  message a Garyx bot (DMs are accepted; groups require @bot), this
  extends transitively: **anyone the owner allows to converse with a
  Garyx bot is trusted to be able to elicit meeting content through the
  agent.** The reader id is a cooperative bookmark key, not an
  authorization boundary. (Confused-deputy exposure is accepted for this
  single-owner deployment; any multi-tenant future requires a real
  authorization design.)
- **S7** Cursor-row cardinality is unbounded (any caller may mint reader
  ids; rows are ≤128-byte keyed, cleaned by entity deletion cascade). For
  a single-principal system the practical reader population is the
  owner's threads; unbounded growth is accepted rather than adding
  quota/GC machinery.
- **S8** Crash model: durability guarantees cover **process exit** (kill,
  panic, restart). Host crash / power loss may additionally lose the
  latest fsynced-but-unsynced-directory artifacts (first-created files,
  fresh index renames); this is accepted for a personal-machine tool and
  is why every recovery path also tolerates missing files.

## 3. Verified repository baseline

| # | Fact | Evidence |
|---|---|---|
| B1 | Feishu WS protobuf data-event path ACKs before processing; a raw-text fallback path processes without that ACK evidence. Only `im.message.receive_v1` handled today; 30 min event-id dedup | `ws.rs:968,997,1040,1049-1072`, `feishu.rs:95` |
| B2 | `FeishuChannel::new(...)` → `FeishuRuntimeContext`; constructor injection precedent | `feishu.rs:370`, `ws.rs:38-69`, `plugin.rs:3235,3322` |
| B3 | gateway depends on channels; channels-defined gateway-implemented trait compiles; first such **production service seam** | `garyx-gateway/Cargo.toml:20` |
| B4 | `FeishuClient` is `Clone` but `pub(crate)` | `client.rs:127` |
| B5 | `CronService` background-service shape | `cron.rs:633,713-796,799-851` |
| B6 | Capsule storage precedent | `capsules.rs:158,273`, `garyx_db/mod.rs:2645,3559` |
| B7 | `GARYX_THREAD_ID` exported to agent processes launched with runtime metadata (normal agent path) | `gary_prompt.rs:148-154` |
| B8 | CLI→gateway pattern; 10 s mutation timeout | `gateway_client.rs:15,73,101,220` |

Platform facts: join/events endpoints as prior revisions; `page_token`
opaque (stored/resumed, never compared); events page_size 20–100; ~30 s
transcript latency (5 s/100-item batches); `10005` not-in-meeting;
5-minute grace; `20001` after. Typed errors:
`NotInMeeting | GraceExpired | AuthFailed | RetriableTransport |
Other(code,msg)`; `AuthFailed` via a fixed code-mapping table (unknown →
`Other`). Console must subscribe the two `vc.bot.*` events.

### 3.1 Sample-pinning gate

Slice 2 opens with sanitized fixtures: invite/ended envelopes (**and the
WS path each arrives on**, per S1), join response, events pages incl.
grace-window reads and `20001`; maximum observed single-page byte size
(validates S4 phrasing). Pinned facts as before; mismatch stops slice 2.

## 4. Entity model and storage

### 4.1 SQLite DDL (normative)

As revision 6 with two changes (R6-05, R6-07): `failure_kind` column and
the corrected list index.

```sql
CREATE TABLE meetings (
  id                   TEXT NOT NULL PRIMARY KEY,
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
  failure_kind         TEXT NOT NULL DEFAULT '' CHECK (failure_kind IN
    ('','auth','transport')),
  failure_since        TEXT,
  end_source           TEXT NOT NULL DEFAULT '' CHECK (end_source IN
    ('','push','poll_ended','grace_expired')),
  join_deadline_at     TEXT NOT NULL,
  grace_deadline_at    TEXT,
  poll_cursor          TEXT NOT NULL DEFAULT '',
  closed_segment_count INTEGER NOT NULL DEFAULT 0 CHECK (closed_segment_count >= 0),
  byte_size            INTEGER NOT NULL DEFAULT 0 CHECK (byte_size >= 0),
  started_at           TEXT NOT NULL,
  ended_at             TEXT,
  finalized_at         TEXT,
  created_at           TEXT NOT NULL,  -- RFC3339 Z, millisecond precision, fixed width
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
CREATE INDEX idx_meetings_created ON meetings(created_at DESC, id DESC);
CREATE INDEX idx_meetings_status  ON meetings(status);
```

`meeting_read_cursors`: unchanged from revision 6 (reader_id 1..128,
pending-triple CHECK, FK cascade). Timestamps: UTC RFC3339 `Z`,
millisecond precision, fixed-width encoding (lexicographic order ==
chronological order, R6-07).

### 4.2 Content log — page-atomic commits (R6-01)

`~/.garyx/meetings/{entity_id}/segments.jsonl`, two line kinds
(`seg` / `ckpt`) as before.

**The commit unit is one platform page.** Each `PollTick` fetches exactly
one page (`poll_events` is one-page-per-call) under a 10 s timeout and
commits it atomically:

1. normalize the page's items into segments (below);
2. under the entity I/O write lock: append the page's `seg` lines →
   append one `ckpt {cursor_out = this page's token}` line → `fdatasync`;
3. SQLite cache transaction.

There is no multi-page tick, no shared-cursor continuation, and no
in-memory carry between pages: the multi-page machinery of earlier
revisions is deleted. `has_more=true` schedules the next page
immediately; otherwise the next poll runs on the 30 s cadence. An
in-flight page dropped by timeout consumes nothing (`cursor_out` stays at
the last committed page).

**Segmentation and record bounds (R6-02):** segments never span pages.
Within a page, consecutive transcript items from one speaker within 60 s
coalesce; every other item kind maps 1:1. Bounds apply to the **final
encoded JSON line**: `speaker` truncated to 256 bytes; `sources` holds
only the item ids of that segment (a coalesced segment lists its own
items — bounded by page item count; a split segment carries exactly its
originating item id); `text` is split at UTF-8 boundaries so each encoded
line ≤ 32 KiB (escape inflation counted, `"cont":true` on
continuations). With every field bounded, a metadata-only overflow cannot
occur; this is asserted by construction tests, not assumed.

**SQLite-cache failure semantics (R6-04):** once the `ckpt` line is
fsynced the page is committed — the coordinator's in-memory cursor
advances unconditionally. A failing cache transaction puts the entity in
a retryable `cache-repair` state (retried with backoff; also healed by
boot repair, which rebuilds caches from the log). The poll never re-pulls
a committed page because of a cache failure; recovery is
forward-only.

**Empty polls** still write a `ckpt` (cursor progress is durable).

**Boot repair** (non-terminal entities only): validate line by line;
truncate everything after the last valid `ckpt` (torn and complete `seg`
lines alike — they belong to an uncommitted page); resume from that
`ckpt`'s token; rebuild caches and the in-memory offset index in the same
pass. Directory/file creation tolerates absence (S8).

### 4.3 Entity deletion

As revision 6 (terminal-only; missing dir = legal empty entity;
tombstone-first rename → DB delete → remove; boot reconciles tombstones
before log repair; confirm/read on deleted → 404).

## 5. Ingestion

### 5.1 Platform seam and registry (R6-05)

Traits as revision 6 (`MeetingPlatformClient` one-page-per-call;
`MeetingEventSink` register/unregister/invited/ended; adapter in
channels; admission insert = 3 bounded retries + nudge, nothing else on
the WS loop). Registry lifecycle contract:

- `register_client` (initial or replace) **immediately nudges every
  non-terminal entity of that account** — recovery cannot depend on
  timers alone.
- `unregister_client` marks affected entities' `stalled_reason =
  'no_client'` visibly.
- Coordinators additionally re-check the registry every 60 s while
  stalled, and **read the current client from the registry at each poll**
  (never caching a stale handle across polls).
- Success on any platform call clears `failure_kind`, `failure_since`,
  and a `stalled_reason` of `auth_failed`/`transport`.

Failure bookkeeping: on a failed platform call the coordinator persists
`failure_kind` (`auth`/`transport`) and `failure_since = now` **iff** the
kind differs from the persisted kind (kind change resets the clock;
R6-05). `stalled_reason` shows when `now - failure_since > 15 min`
(mapped per kind) or immediately for `no_client`.

### 5.2 Coordinator

As revision 6 with the page-atomic tick: commands are processed
immediately when no page fetch/commit is executing; a command arriving
during one is handled right after that page's commit (bound: 10 s fetch
timeout + one durability flush). Everything else — admission → joining →
live, absolute join deadline (applies even stalled), intent-staged
terminals (`aborting`, `finalizing`), end > abort arbitration,
`NotInMeeting`/`GraceExpired` semantics, grace drain to the deadline with
no quiescence shortcut, index persistence inside the terminal intent,
unknown-entity end signals dropped, boot recovery per persisted stage —
is unchanged from revision 6.

### 5.3 State machine

Unchanged from revision 6 (JOINING → LIVE → FINALIZING → FINALIZED;
ABORTING → ABORTED; the tick label now means "page commit").

## 6. Read protocol

### 6.1 Surfaces

As revision 6. `--max-bytes` has a floor of 4096; below it the CLI errors
(R6-02). A response always makes progress: it either serves ≥1 segment
line (single lines are ≤32 KiB < any legal budget) or states completion;
no non-advancing continuation token is ever returned.

### 6.2 List pagination (R6-07)

Ordered `(created_at DESC, id DESC)` (index matches). Keyset semantics
are **explicitly weak for concurrent inserts**: rows that sort after the
current cursor position — e.g. a clock-skewed smaller `created_at`, or a
same-timestamp smaller id — will appear in later pages of an in-progress
traversal; rows sorting before it will not. No snapshot machinery; a
gallery refresh re-lists. Updates never move rows (immutable key);
deletion during traversal simply omits the row.

### 6.3 Locking, snapshots, offset index (R6-04)

As revision 6, plus: terminal index rebuild is **single-flight** and runs
as a detached background task owned by the service (not the HTTP
handler); a read that needs a missing/invalid index triggers the rebuild
and returns a retryable `index_building` response if it cannot complete
within its budget; the CLI retries with backoff (and says so). Concurrent
cold reads share the single rebuild. Directory-entry durability for the
rename is best-effort under S8.

### 6.4 Incremental fetch/confirm

Unchanged from revision 6 (linearized ensure-row → claim-or-adopt →
confirm; shared receipts; crash-before-flush re-serves; confirm-committed-
response-lost moves on; never regress; deleted entity → 404).

### 6.5 Stateless snapshots (R6-08)

Each page of a `--full`/`--range` stream returns a **fresh token for the
same pinned snapshot** with a sliding 10-minute expiry (expiry measures
inactivity, not total duration). A printed resume command therefore works
if run within 10 minutes of the last served page; after that the read
restarts (stateless by design; stated in the output). Tokens are
URL/shell-safe base64url and carry
`{entity_id, snapshot, next_seq, mode, range_end, checksum, issued_at}`.

### 6.6 Self-describing output and untrusted framing (R6-03)

All platform-derived text (topic, speaker names, transcript text, share
titles) is **untrusted data** everywhere it is rendered — CLI and API
alike. Human-format output uses unambiguous physical framing: every
content line is prefixed with `│ `, and metadata/header lines never share
a line with content; control characters (C0 except `\n` in content
bodies, C1, ANSI CSI/OSC introducers) are stripped from platform text at
render time; Bidi controls are replaced with their escaped codepoint
form. `--json` emits platform text only as JSON string values. Headers
are synthesized from structured fields; nothing in a content body can
open a new header or fake a segment boundary. The design makes no
"trusted output" claim anywhere.

## 7. Conversation references: a text convention

As revision 6: the prefill line contains only the entity id and the fixed
read command (no topic, no platform text). The CLI's own rendering of
topic/status on read is governed by 6.6 untrusted framing. Recognition =
cursor row; discovery = `garyx meeting list`; deleted entities fail
clearly; stale lines are harmless.

## 8. UI surfaces (slice 3)

As revision 6 (galleries, keyset list, 30 s visible polling, prefill
action, packaged validation). Platform text in gallery cells is rendered
as plain text (no markup interpretation), consistent with 6.6.

## 9. Configuration and deployment prerequisites

As revision 6 (two config fields; console event subscriptions; sanitized
fixtures).

## 10. Failure and observability

As revision 6, plus `failure_kind` in logs/list output, `cache-repair`
state logging, and `index_building` responses logged at debug.

## 11. Implementation impact map

As revision 6 (all additive; bridge/providers/router/transcripts/
render_state/SSE/thread lifecycle untouched).

## 12. Test plan

Revision 6's matrix, amended:

- **Removed** (R6-09): the oversized same-cursor multi-tick expectation —
  the mechanism no longer exists.
- **Page-atomic commits**: crash after `seg` lines before `ckpt` (whole
  page re-pulled, no dupes via truncation); crash after `ckpt` before
  SQLite (cache-repair, no re-pull, forward-only); in-flight page timeout
  consumes no cursor; `has_more` immediate rescheduling; empty-poll ckpt.
- **Record bounds** (R6-02): metadata-only overflow impossible by
  construction (property test over max-size speaker/sources/items);
  escape-inflation splits; `--max-bytes` floor rejection (0/1/4095);
  single-line-per-response minimum progress.
- **Trust/framing** (R6-03): hostile topic/speaker/transcript with
  header-lookalikes, ANSI/OSC sequences, Bidi controls, C1 bytes —
  asserted stripped/framed in human output and safely encoded in
  `--json`; cross-conversation elicitation documented as S6 (no test can
  "pass" a product sign-off, but the fixture demonstrates the accepted
  behavior for the record).
- **Cache-failure** (R6-04): fdatasync-ok → SQLite-fail → continue
  polling forward + boot repair heals caches; restart in cache-repair
  state.
- **Registry/failure-kind** (R6-05): transport 14 min → auth 2 min shows
  auth with a reset clock; register-nudge recovers stalled entities
  without waiting for timers; disable→enable; per-poll registry read
  observes a replaced client; finalizing with no client finalizes at
  deadline.
- **Cursor cardinality** (S7): documented-behavior fixture (many reader
  ids), cascade cleanup on delete.
- **List** (R6-07): same-timestamp smaller-id insert appears in a later
  page (weak semantics demonstrated); clock-skew insert; unreturned-row
  deletion.
- **Snapshot tokens** (R6-08): sliding renewal across >10 min total
  stream; resume after delay <10 min works; >10 min restarts with clear
  message; token shell-safety.
- **Raw-text WS path** (S1/R6-09): fixture asserting which path carries
  invite/ended (gate outcome recorded); if raw-text carries them, slice 2
  halts (test asserts the halt condition, not a fabricated semantics).
- **Index rebuild** (R6-04): single-flight under concurrent cold reads;
  client-timeout-then-retry hits the completed index; `index_building`
  retryable response.

Scope: `cargo test --all-targets` for `garyx-channels`, `garyx-gateway`,
`garyx-models`, `garyx`; tier1 fast loop; slice 3 adds SwiftPM +
`xcodebuild` + packaged desktop.

## 13. Delivery slices

As revision 6 (entity core + read protocol → ingestion behind the sample
gate → experience), with slice 1's gate extended by the record-bounds and
framing suites.

## 14. Open questions (defaults chosen, non-blocking)

As revision 6; plus: quota/GC for cursor rows and any real authorization
model are explicitly future work gated on S6/S7 being revisited.
