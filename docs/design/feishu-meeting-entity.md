# Feishu meeting entity

Status: revision 10 — complete self-contained specification (no references
to prior revisions anywhere), addressing adversarial review #TASK-2337
rounds 1–9.
Author: gary (design)
Scope ruling (user, 2026-07-16): orthogonal side-system; existing runtime
flows are not modified; agents read via CLI; conversation references are
plain text; read cursors live on the read side.
Supersedes: `docs/design/feishu-group-listen-mode.md` (group listening
cancelled; meeting content never enters thread ledgers).

## 1. Summary

When someone invites the Garyx bot into an ongoing Feishu meeting, the bot
joins, polls in-meeting events, and materializes a **meeting entity** in a
self-contained subsystem: its own SQLite tables, on-disk segment log,
gateway routes, and CLI. Capture commits **one platform page at a time**,
atomically with its checkpoint. When the meeting ends, capture drains the
platform grace window and the entity becomes immutable.

Agents read entities exclusively through `garyx meeting read`: per-reader
server cursors, linearized fetch/confirm, at-least-once delivery,
self-describing output, and untrusted handling of all platform text.

## 2. Product contract (user-approved 2026-07-16; scope reset same day)

1. Meetings only. 2. Invite-driven join (+ debug `garyx meeting join`).
3. Live near-real-time accumulation (~30 s platform latency). 4. CLI-only
reads, per-reader cursors, incremental default, self-describing. 5. End
signal freezes the entity. 6. The cursor row is the recognition of prior
reference. 7. Orthogonality: additive touchpoints only (Section 11).

Product sign-off items (explicit, owner-revocable):

- **S1** On the Feishu WS **protobuf data-event path**, events are ACKed
  before processing; an invite on that path is lost iff the process dies
  or the admission insert exhausts 3 bounded retries (100 ms / 1 s
  backoff, error-logged) in the ACK→admission window. The raw-text
  fallback path has no equivalent ACK evidence; the sample gate (3.1)
  pins which path carries invite/ended events; if the raw-text path can
  carry them, slice 2 halts for a design amendment.
- **S2** One entity per admission: re-invites after terminal state or
  deletion create new entities capturing from that point.
- **S3** The final ~30 s of speech becomes readable during the grace
  drain, not instantly.
- **S4** A crash normally loses nothing (an uncommitted page is
  re-pulled). If the platform becomes unreadable before the re-pull
  (grace expired while down; bot removed), at most **one platform events
  page** (page_size ≤ 100 items) is permanently, silently lost.
- **S5** Agents learn about entities from referenced text,
  `garyx meeting list`, or memory; no automatic per-turn injection; an
  unreferenced entity may go unnoticed.
- **S6** Access/trust: Garyx is a single-principal personal system. Every
  gateway-authenticated caller can read the full meeting catalog and all
  content. Agents act on behalf of anyone who can message a Garyx bot
  (DMs accepted; groups require @bot), so **anyone the owner allows to
  converse with a Garyx bot is trusted to be able to elicit meeting
  content through the agent** (confused-deputy exposure accepted for
  single-owner deployment; multi-tenant futures need a real authorization
  design).
- **S7** Cursor-row cardinality is unbounded (any caller may mint reader
  ids; each row's key is ≤128 **bytes**; rows die with their entity).
  Accepted instead of quota/GC machinery.
- **S8** Crash model: durability guarantees cover **process exit**. Host
  crash / power loss may additionally lose first-created files or fresh
  renames (unsynced directory entries); accepted for a personal-machine
  tool; every recovery path tolerates missing files.

## 3. Verified repository baseline

| # | Fact | Evidence |
|---|---|---|
| B1 | Feishu WS protobuf data-event path ACKs before processing; a raw-text fallback path processes without that ACK evidence. Only `im.message.receive_v1` is handled today; other event types fall to an ignore branch; 30 min in-memory event-id dedup | `garyx-channels/src/feishu/ws.rs:968,997,1040,1049-1072`, `feishu.rs:95` |
| B2 | `FeishuChannel::new(config, router, bridge, dispatcher, public_url)`; per-account deps flow into `FeishuRuntimeContext`; constructor injection threads through `BuiltInPluginDiscoverer::with_dispatcher` | `feishu.rs:370`, `ws.rs:38-69`, `plugin.rs:3235,3322` |
| B3 | gateway depends on channels (never reverse); a channels-defined, gateway-implemented trait compiles; first such **production service seam** | `garyx-gateway/Cargo.toml:20` |
| B4 | `FeishuClient` owns tenant-token refresh (double-checked, single-flight, 5 min margin) and is `Clone`, but `pub(crate)` — gateway cannot name it; cross-crate use requires a public trait object | `client.rs:127,226,243` |
| B5 | Long-lived background-service precedent: `CronService` — select loop + stop channel + `Weak<AppState>` backref + stale-state reset on boot | `cron.rs:633,713-796,799-851` |
| B6 | Entity storage precedent: capsules — content on disk + STRICT SQLite metadata + PRAGMA column migration | `capsules.rs:158,273`, `garyx_db/mod.rs:325,2645,3559` |
| B7 | `GARYX_THREAD_ID` is exported to agent processes launched with runtime metadata (the normal agent path); CLI precedent reads it via `env_nonempty` | `gary_prompt.rs:148-154`, `commands/task.rs:233`, `gateway_client.rs:439` |
| B8 | CLI→gateway pattern: `gateway_endpoint` (base URL + bearer from `garyx.json`) + shared retrying JSON helpers; 10 s mutation timeout | `gateway_client.rs:15,73,101,220` |

External platform facts (lark-cli 1.0.70 + internal onboarding manual;
scopes granted and live-verified on the production app):

- `POST /open-apis/vc/v1/bots/join`: 9-digit `meeting_no` + optional
  `call_id` → long numeric `meeting.id`. Tenant token only.
- `GET /open-apis/vc/v1/bots/events`: pull; `page_token` continuation —
  **opaque**: stored and resumed from, never compared or ordered;
  page_size 20–100.
- Event types: `participant_joined/left`, `chat_received`,
  `transcript_received`, `magic_share_started/ended`
  (`share_doc.title/url`).
- Transcript ~30 s latency, batched 5 s/100 items. Multi-party meetings
  only. Per-meeting owner switch "allow agents to join". `10005` = bot
  not in meeting. 5-minute post-end grace window; `20001` = window over.
- Post-meeting minutes are not auto-authorized to the bot; not a
  dependency.
- Console must subscribe `vc.bot.meeting_invited_v1` and
  `vc.bot.meeting_ended_v1` (deployment prerequisite; misspelled
  subscriptions silently receive nothing).

Typed platform errors:
`MeetingApiError = NotInMeeting | GraceExpired | AuthFailed |
RetriableTransport | Other(code, msg)`. `AuthFailed` via a fixed mapping
table from Feishu auth-class codes (populated at implementation from the
official error table; unknown codes default to `Other`).

### 3.1 Sample-pinning gate

Slice 2 opens by capturing sanitized fixtures: invite/ended envelopes
**and the WS path each arrives on** (S1), join response, events pages
including grace-window reads and the `20001` response, and the maximum
observed single-page byte size (validates S4 phrasing). Pinned facts:
invite payload fields; joining-stage identity; existence or absence of an
in-band ended item (if absent, end sources are exactly `push` and
`grace_expired`); grace-window read behavior. Any mismatch stops slice 2
for a design amendment. Slice 1 has no Feishu dependency.

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
  topic                TEXT NOT NULL DEFAULT '',        -- normalized: ≤256 bytes, UTF-8 boundary
  invited_by           TEXT NOT NULL DEFAULT '',        -- ≤128 bytes
  status               TEXT NOT NULL CHECK (status IN
    ('joining','live','finalizing','aborting','finalized','aborted')),
  status_detail        TEXT NOT NULL DEFAULT '',        -- ≤256 bytes
  content_state        TEXT NOT NULL DEFAULT 'ok' CHECK (content_state IN ('ok','lost')),
  content_lost_at      TEXT,
  failure_kind         TEXT NOT NULL DEFAULT '' CHECK (failure_kind IN
    ('','auth','transport')),
  failure_since        TEXT,
  log_epoch            INTEGER NOT NULL DEFAULT 0 CHECK (log_epoch >= 0),
  cache_generation     INTEGER NOT NULL DEFAULT 0 CHECK (cache_generation >= 0),
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
  updated_at           TEXT NOT NULL,
  CHECK ((failure_kind = '') = (failure_since IS NULL))
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

CREATE TABLE meeting_read_cursors (
  meeting_id    TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  reader_id     TEXT NOT NULL
    CHECK (length(CAST(reader_id AS BLOB)) BETWEEN 1 AND 128),  -- bytes, not chars
  log_epoch     INTEGER NOT NULL DEFAULT 0 CHECK (log_epoch >= 0),   -- RR9-01
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

Notes:

- All timestamps: UTC RFC3339 `Z`, millisecond precision, fixed-width
  encoding (lexicographic == chronological).
- `stalled_reason` is **not stored**: `no_client` is derived live from
  the registry; `auth_failed`/`transport` derive from
  `failure_kind`+`failure_since` (Section 5.1). List output synthesizes
  the field.
- `reader_id` = trimmed `GARYX_THREAD_ID` or `--thread` value; a
  cooperative bookmark key (S6/S7), never validated against
  `thread_records`; no thread-lifecycle hooks; rows die with the entity.
- There is no refs table; recognition is the cursor row.

### 4.2 Content log: page-atomic commits (normative)

`~/.garyx/meetings/{entity_id}/segments.jsonl`
(`default_meetings_dir()` beside `default_capsules_dir()`), two line
kinds:

```
{"t":"seg","seq":12,"kind":"transcript","speaker":"张三","start":"…","end":"…",
 "text":"…","sources":["sent_8813"],"cont":false}
{"t":"ckpt","epoch":1,"cursor_out":"pt_x9y8…","at":"2026-07-16T02:35:12.123Z"}
```

**Log epoch (RR8-02, RR9-01):** a log file's lifetime is one epoch. When
boot (or a writer's `ensure_dir`) finds the log **missing** for an entity
whose DB row says `cache_generation > 0`, the loss is explicit, in one
transaction (boot-exclusive, or under the entity write lock at runtime;
logged loudly):

- increment `meetings.log_epoch`; reset `cache_generation = 0`,
  `poll_cursor = ''`, `closed_segment_count = 0`, `byte_size = 0`;
- **invalidate the read domain**: for every cursor row of the entity,
  set `log_epoch = new epoch, confirmed_seq = 0, pending_from/to = NULL,
  receipt = NULL` (recognition survives; positions do not — old seqs
  name content that no longer exists);
- at runtime additionally reset the coordinator's in-memory poll cursor,
  next-seq counter, coalescing state, and offset index.

Every cache update and repair is guarded by
`WHERE id=:id AND log_epoch = :epoch AND cache_generation < :gen`.
Seqs restart at 1 in a new epoch (seq alone is never a cross-epoch
identity): snapshot tokens, `index.bin` headers, read snapshots, and
every rendered span carry the epoch; confirm CAS matches
`(log_epoch, confirmed_seq, pending, receipt)`; a stale-epoch snapshot
token or confirm returns an explicit `snapshot invalidated by content
loss` error; a `--range` request may carry an optional epoch (default:
current) and a non-current epoch returns the same content-loss error.
Boot repair verifies every `ckpt` line's epoch equals the DB's current
epoch (foreign-epoch lines are treated as an invalid tail). Field-bound
table: `epoch` is a non-negative integer, bounded by rollover count.

A **terminal** entity whose non-empty log is found missing at read time
gets `content_state='lost'` + `content_lost_at=now` — **separate
structured columns** (`content_state TEXT NOT NULL DEFAULT 'ok'
CHECK (content_state IN ('ok','lost'))`, `content_lost_at TEXT`), never
overwriting the lifecycle `status_detail` (RR9-04). Detection happens on
the read path but marks idempotently **under the entity write lock**
(serialized against delete; a deleted entity wins with 404). Reads of a
lost entity return an explicit `content lost` error (never stale totals);
metadata endpoints surface both columns.

**Field bounds (normative; every field, R7-03):**

| Field | Bound / form |
|---|---|
| `t` | `"seg"` \| `"ckpt"` |
| `seq` | dense positive integer, assigned at append |
| `kind` | enum: `transcript`, `chat`, `share_start`, `share_end`, `join`, `leave` |
| `speaker` | ≤256 bytes, truncated at UTF-8 character boundary |
| `start`/`end`/`at` | fixed-width RFC3339 ms `Z` (24 bytes) |
| `text` | split so the **final encoded JSON line** ≤ 32 KiB (escape inflation counted) |
| `sources` | array of item ids; each stored id ≤**71 bytes** — raw ids ≤64 bytes pass through; longer ids are replaced by `sha256:<hex>` (7-byte prefix + 64 hex = exactly 71 bytes; deterministic, domain-separated, collision-safe for dedup); array bounded by page item count. Property tests assert the normalized field's own byte length, not merely the whole-line bound |
| `cont` | boolean |
| `cursor_out` | opaque token, ≤1 KiB (longer is a platform-contract violation → tick fails as `Other`) |
| share titles / URLs (inside `text` for share kinds) | title ≤256 bytes UTF-8-boundary truncated; URL ≤1 KiB |

**Segmentation (order matters, R7-03):** items are first mapped 1:1 to
candidate segments (each carrying exactly its own item id in `sources`).
Adjacent transcript candidates from the same speaker within 60 s are then
coalesced **only if** the merged candidate's final encoded line stays
≤ 32 KiB (a coalesced segment's `sources` lists its member item ids).
Oversized `text` is split at UTF-8 boundaries into consecutive seqs with
`"cont":true`; each continuation carries the same single originating item
id (splitting applies only to 1:1 candidates — a merge that would need
splitting is simply not merged, so continuation source mapping is always
unique). Segments never span pages.

**Page-atomic commit (the durability core):** each poll fetches exactly
one page (`poll_events` is one-page-per-call) under a 10 s timeout and
commits atomically under the entity I/O write lock:

1. normalize the page → `seg` lines (bounded as above);
2. append `seg` lines → append one `ckpt {cursor_out = this page's
   token}` → `fdatasync`;
3. SQLite cache transaction guarded by **epoch + monotonic generation**
   (R7-04, RR8-02): `UPDATE meetings SET poll_cursor=…,
   closed_segment_count=…, byte_size=…, cache_generation=:gen,
   updated_at=… WHERE id=:id AND log_epoch = :epoch AND
   cache_generation < :gen` — `:gen` is the checkpoint ordinal within the
   current epoch, so a delayed repair can never overwrite a newer cache
   and a stale-epoch writer can never touch a re-created log.

Once the `ckpt` is fsynced the page is committed: the coordinator's
in-memory cursor advances unconditionally; a failed cache transaction
puts the entity in a retryable `cache-repair` state (backoff retries;
each repair re-derives values from the canonical log **under the I/O
lock** with the current generation guard). Recovery is forward-only: a
committed page is never re-pulled because of cache failure. Empty polls
still write a `ckpt`. `has_more=true` schedules the next page
immediately; otherwise the 30 s cadence (jittered) applies. An in-flight
page dropped by its timeout consumes nothing.

**Boot repair** (non-terminal entities only; terminal logs are never
scanned at boot): validate line by line; truncate everything after the
last valid `ckpt` (torn and complete `seg` lines alike — they belong to
an uncommitted page); resume from that `ckpt`'s token; rebuild SQLite
caches (with the generation guard) and the in-memory offset index in the
same pass. Missing directories/files are tolerated everywhere (S8).

### 4.3 Entity deletion

Terminal entities only (409 otherwise). Under the entity I/O write lock:

1. wait for / cancel any in-flight index rebuild for the entity (R7-04);
2. atomic `rename {id}/ → {id}.tombstone/` — a missing dir is a legal
   empty entity (a joining-deadline abort may never have appended):
   skip the rename;
3. DB transaction deletes the `meetings` row (cascades cursors);
4. remove the tombstone dir.

Boot order: **tombstone reconcile before log repair** — tombstone+row →
rename back (delete never committed); tombstone without row → remove;
bare dir without row → remove (logged). Confirm/read on a deleted entity
→ 404; the CLI reports "entity deleted".

API: `DELETE /api/meetings/{id}`; CLI `garyx meeting delete <id>`.

## 5. Ingestion

### 5.1 Platform seam, registry, failure bookkeeping

`garyx-channels/src/meeting_sink.rs` (new):

```rust
#[async_trait]
pub trait MeetingPlatformClient: Send + Sync {
    async fn join(&self, meeting_no: &str, call_id: Option<&str>)
        -> Result<JoinedMeeting, MeetingApiError>;
    async fn poll_events(&self, feishu_meeting_id: &str, page_token: &str)
        -> Result<EventsPage, MeetingApiError>;   // exactly one page per call
}
pub trait MeetingEventSink: Send + Sync {
    fn register_client(&self, account_id: &str, client: Arc<dyn MeetingPlatformClient>);
    fn unregister_client(&self, account_id: &str);
    fn on_meeting_invited(&self, invite: MeetingInvite);
    fn on_meeting_ended(&self, account_id: &str, feishu_meeting_id: &str);
}
```

- Adapter over `FeishuClient` lives in garyx-channels (B4).
  `FeishuChannel::start` registers per enabled account; `stop` and
  account disable/removal unregister; re-register replaces. A no-op sink
  keeps other assemblies compiling.
- `on_meeting_invited` = admission insert (3 bounded retries, S1) +
  coordinator nudge; nothing else on the WS loop.
- **Registry is the source of `no_client`** (R7-05): the condition is
  derived live (`registry.get(account_id).is_none()`), never persisted.
  `register_client` (initial or replace) immediately nudges every
  non-terminal entity of that account; coordinators also re-check every
  60 s while lacking a client, and **fetch the current client from the
  registry at each poll** (no caching across polls).
- **Failure bookkeeping** (persisted; R7-05): on a failed platform call,
  if the error's kind (`auth`/`transport`) differs from the persisted
  `failure_kind`, set `failure_kind = new kind, failure_since = now`
  (kind change resets the clock — a clock never spans kinds). Entering or
  leaving the no-client condition also resets the pair (`'' / NULL` on
  unregister-observed, fresh start on next failure after re-register —
  continuity across an unregister gap is never assumed). On **any**
  successful platform call, clear `failure_kind`/`failure_since`. The
  synthesized `stalled_reason` is computed **only for states that depend
  on a platform client — `joining`, `live`, `finalizing`** (RR8-05):
  `no_client` if the registry lacks the client; else
  `auth_failed`/`transport` if `now - failure_since > 15 min`; else
  empty. Entities in `aborting` or terminal states never synthesize a
  stalled reason, and entering `aborting` or a terminal state clears the
  `failure_kind`/`failure_since` pair (no stale failure survives into
  history).

### 5.2 Coordinator over durable intents

`MeetingService` (gateway; CronService shape B5: `Arc` on AppState, stop
channel, `Weak<AppState>`): one coordinator task per non-terminal entity
consuming a command queue (`Nudge`, `EndedSignal(source)`,
`AbortRequest`, `Shutdown`, self-scheduled `PollTick`).

**Command scheduling:** commands are processed immediately whenever no
page fetch/commit is executing; a command arriving during one is handled
right after that page's commit (bound: 10 s fetch timeout + one
durability flush). A joining or stalled entity (no polls running) handles
admin aborts instantly.

**Durable CAS:** every transition is
`UPDATE meetings SET status=:to … WHERE id=:id AND status=:from`; losers
are no-ops. Intent stages make terminals crash-resumable:

- finalize: `live → finalizing` (drain runs here) → **cache/index
  barrier** (R7-04, RR8-02): derive the final cache values from the
  canonical log under the I/O lock, write them, then **read back and
  verify** that the row's `(log_epoch, cache_generation, counts)` match
  the derivation — a zero-row guarded UPDATE is *not* success; a
  mismatch re-derives and retries. The offset-index persistence (6.3)
  must also succeed (an entity whose log is legally empty — e.g. a
  joining-deadline abort — persists a valid empty index). Only then
  `finalizing → finalized`.
- abort: `joining|live → aborting` (`status_detail` set) → final `ckpt`
  if a page commit was interrupted → same read-back-verified barrier →
  `aborting → aborted`.
- Terminal-affecting inputs (`EndedSignal`, `AbortRequest`,
  `NotInMeeting`) queued together resolve at the next scheduling point
  with priority **end > abort**.
- `finalized`/`aborted` refuse appends (checked under the I/O lock).
- Boot resumes each persisted stage (joining retries; live polls;
  finalizing drains to its deadline; aborting finishes its flush).

**Admission → joining:** the sink insert creates `status='joining'`,
`join_deadline_at = now + join_retry_window` (absolute). Unique indexes
make duplicate invites no-ops (S2). Content dir is created lazily by the
writer (`ensure_dir` before first append). Join retries every 20 s until
success — CAS `joining→live`, backfill `feishu_meeting_id` (normalized)
and `topic` (normalized per 4.1 bounds) — or until the deadline → abort
path. The deadline applies **even while stalled** (it models invite
validity). Join succeeding during the grace window is legal; polls then
return data until `GraceExpired` finalizes.

**Live polling:** page-atomic commits per 4.2. Transport errors back off
30→60→120 s. `NotInMeeting` while live (no end signal) → abort path.
`GraceExpired` while live → finalize immediately
(`end_source='grace_expired'`).

### 5.3 End path and grace drain

`EndedSignal(push)` (or `poll_ended` if pinned by 3.1): CAS
`live→finalizing`, `ended_at=now`, `grace_deadline_at = now + 4 min`.
Drain: keep polling on the normal cadence **until the deadline** — no
quiescence shortcut. Then the terminal barrier and CAS `finalized`.
`GraceExpired` during finalizing accelerates completion; `NotInMeeting`
during finalizing completes early; `AbortRequest` during finalizing is
refused; a stalled finalizing entity still finalizes at its deadline.
Unknown-entity end signals are logged and dropped (grace-window polls are
the correlation-free backstop). Restart during finalizing resumes the
drain to the persisted deadline.

### 5.4 State machine

```
 invite ─admission insert─> JOINING ─join ok─> LIVE ─page-atomic commits─┐
   │      (unique keys, S1/S2)  │(absolute        │                      │ commands: immediate
   │                            │ deadline,       │ EndedSignal/         │ when idle; after the
   │                            │ even stalled)   │ GraceExpired         │ current page commit
   │                            v                 v                      │ when executing
   │                        ABORTING ─flush+barrier─> ABORTED            │ (end > abort)
   │                            ^                                        │
   │                            └────10005 while live────────────────────┤
   │                                                                     v
   │                                                    FINALIZING ─drain to deadline;
   │                                                     │ GraceExpired/10005 ⇒ complete
   │                                                     │ early; abort refused; stalled ⇒
   │                                                     │ finalize at deadline;
   │                                                     │ cache/index barrier here
   │                                                     v
   │                                                FINALIZED
 boot: tombstone reconcile → log repair (non-terminal only) → coordinators
 resume persisted stages; missing client ⇒ visible no_client + 60 s
 re-check + register-nudge; admin abort applies to joining|live.
```

## 6. Read protocol

### 6.1 Surfaces

CLI:

- `garyx meeting list [--json]`
- `garyx meeting read <id> [--full | --range A..B] [--thread <id>]
  [--json] [--max-bytes N]`
- `garyx meeting join <meeting_no> [--account <id>]` (debug)
- `garyx meeting abort <id>` (admin; joining|live)
- `garyx meeting delete <id>` (terminal only)

API:

```
GET    /api/meetings?limit&page_token      -> keyset-paged list
GET    /api/meetings/{id}                  -> metadata
POST   /api/meetings/{id}/read             -> fetch (incremental|full|range)
POST   /api/meetings/{id}/read/confirm     -> confirm {receipt}
POST   /api/meetings/{id}/abort            -> admin abort
DELETE /api/meetings/{id}                  -> delete
POST   /api/meetings/{id}/join-debug       -> manual trigger
```

Incremental (default) requires a reader identity (`GARYX_THREAD_ID` env
or `--thread`; missing both → error naming both remedies). `--full` and
`--range` are stateless paged snapshot peeks: no identity, no cursor rows
ever created or touched. Only a successful incremental fetch creates a
cursor row.

**Response budget (R7-02, RR8-01):** `--max-bytes` (floor 4096; below →
CLI error) is a **soft target measured on the final rendered response**
(headers and framing included, per mode). Two rules govern its
interaction with pending spans:

- **Every newly produced page is budget-bounded server-side:** the
  claimed span (incremental) and every `--full`/`--range` snapshot page
  are sized to `min(requested_max, read_page_bytes)` — `read_page_bytes`
  is the **server-side hard cap** on any newly produced page (RR9-03: a
  huge `--max-bytes` cannot bypass server pagination), so no caller can
  mint an arbitrarily large pending span or response — with the single-segment
  minimum-progress exception (a lone segment whose rendered form exceeds
  the budget is still claimed and served, with the explicit header note
  `single segment exceeds requested budget`).
- **Pending replay is indivisible:** an existing pending span is the
  atomic delivery unit. A re-serve returns the **entire pending span**
  regardless of the current request's budget or render mode (header
  note: `pending replay exceeds requested budget`), because serving a
  subset while confirming the original receipt would silently skip the
  remainder. Budget and mode apply to *new* claims only.

A zero-progress response or non-advancing continuation token is never
returned; response
metadata (headers, topic line) is counted before content.

### 6.2 List pagination

Ordered `(created_at DESC, id DESC)` (matching index). Keyset semantics
are **explicitly weak for concurrent inserts**: rows sorting after the
current cursor position (clock-skewed smaller `created_at`;
same-timestamp smaller id) appear in later pages; rows sorting before it
do not appear in an in-progress traversal. Updates never move rows
(immutable key); deleted rows are omitted. A gallery refresh re-lists.

### 6.3 Locking, snapshots, offset index

- **Entity I/O lock:** per-entity `RwLock` in a service map, covering all
  states. Writers (page commits, terminal flush/barrier, delete) take
  write; reads take read and capture a snapshot
  `(closed_latest, log_byte_offset)`; slicing never scans past the
  snapshot offset. Terminal entities take the lock directly; live
  entities' snapshots are requested through the coordinator.
- **Sparse offset index** (every 64th seq → byte offset). Live: built
  during boot repair, maintained per append, memory-only. Terminal:
  persisted `{id}/index.bin` written inside the terminal barrier
  (5.2) — temp-file → `fdatasync` → atomic rename; header
  `{version, log_epoch, log_byte_len, latest_seq, crc32}` (epoch mismatch = invalid, RR9-01). On read, a missing,
  torn, version-mismatched, or length/crc-mismatched index is discarded
  and rebuilt **single-flight** by a detached service-owned background
  task that holds the entity's read lock and re-verifies the entity row
  (not a tombstone) before the rename; a read that cannot complete
  within its budget returns retryable `index_building` and the CLI
  retries with backoff. Delete waits for / cancels in-flight rebuilds
  (4.3).

### 6.4 Incremental fetch/confirm (linearized)

Cursor algebra (single SQLite transactions):

0. **Preflight (RR8-03):** the read snapshot and (for terminal entities)
   a valid offset index must be available **before** any cursor write. A
   read that would return `index_building`, `content lost`, or any error
   exits here — a failed first read creates no recognition state. (Once
   a pending span exists, a later lost response of course leaves the row
   in place — that is the at-least-once path, not a failed first read.)
1. **Ensure row:** `INSERT INTO meeting_read_cursors (meeting_id,
   reader_id, confirmed_seq, updated_at) VALUES (…, 0, now)
   ON CONFLICT DO NOTHING` — created on the first incremental fetch that
   passes preflight, even if the increment is empty (the row is the
   recognition state).
2. **Fetch:** read the row. Pending exists → re-serve exactly
   `(pending_from..pending_to)` with the stored receipt (at-least-once;
   receipts are shared, not per-caller). Else compute the next span
   `(confirmed_seq, min(latest_snapshot, budget-limited end))` and claim:
   `UPDATE … SET pending_from=:f, pending_to=:t, receipt=:r, updated_at=now
   WHERE meeting_id=:m AND reader_id=:rd AND confirmed_seq=:expected
   AND pending_from IS NULL`. Zero rows → a concurrent fetch won →
   re-read the row and **re-run preflight until the snapshot covers the
   winner's `pending_to`** (RR9-02: the loser's earlier snapshot may
   predate the winner's claim; slicing never exceeds the snapshot, so
   the loser refreshes rather than violating it), then re-serve the
   winner's span and receipt. An empty span
   serves the empty-increment header without claiming pending.
3. **Confirm** (`/read/confirm {receipt}`):
   `UPDATE … SET confirmed_seq=pending_to, pending_from=NULL,
   pending_to=NULL, receipt=NULL, updated_at=now
   WHERE meeting_id=:m AND reader_id=:rd AND receipt=:receipt
   AND log_epoch=:current_epoch` (a rollover between fetch and confirm
   fails the CAS; the CLI reports the content-loss error).
   Unknown/stale receipt → idempotent no-op success. Deleted entity →
   404.
4. **The CLI performs fetch → render → stdout flush → confirm in one
   invocation.** Crash/broken pipe before flush → pending survives → the
   same span re-serves next time. Confirm committed but its response
   lost → pending already cleared → the next read serves the next span
   (safe: content was flushed before confirm was attempted). Cursors
   never regress; row CAS serializes concurrent readers.

### 6.5 Stateless snapshots (`--full`, `--range`)

The first response pins `(closed_latest, log_offset)`; every page returns
a **fresh token for the same snapshot** with a sliding 10-minute
inactivity expiry. Tokens are base64url (shell-safe) encoding
`{entity_id, log_epoch, snapshot, next_seq, mode, range_end, checksum, issued_at}` — a token whose epoch is no longer current fails with `snapshot invalidated by content loss`.
The CLI loops within one invocation streaming pages to stdout until
exhausted, or stops at `--max-bytes` and prints the resume command with
the latest token. Inactivity >10 min → the read restarts from the
beginning (stateless by design; stated in output). Appends beyond the
snapshot are invisible to that snapshot's pages.

### 6.6 Self-describing output and untrusted framing

All platform-derived text (topic, speaker, transcript text, share
titles) is **untrusted data** wherever rendered. Human format: every
content line is prefixed `│ `; metadata/header lines never share a line
with content; **Unicode forced line breaks `U+2028`/`U+2029` are
normalized to LF before per-line prefixing** (RR8-06), then C0 (except
`\n` in bodies) / C1 / ANSI CSI+OSC introducers are stripped from
platform text at render time; Bidi controls are replaced by escaped
codepoint forms. `--json` emits platform text only as JSON string
values. Headers are synthesized from structured fields; log content
cannot forge a header or segment boundary — no unprefixed line can
originate from platform text. No "trusted output" claim exists anywhere
in this design.

Every response names: mode; exact span served; totals; entity status —
covering **all DDL states**: joining / live / finalizing / aborting /
finalized / aborted (+ `end_source`, synthesized `stalled_reason` where
applicable, `content_lost` marker, timestamps); for incremental —
confirmation state and "re-read any span: `--range A..B`"; for empty
increments — "no new segments since [N]"; for first reads — "first read
for this reader"; for budget overshoot / pending replay — the explicit
notes (6.1).

## 7. Conversation references: a text convention

A reference is a line of text containing the **entity id and the fixed
read command only** — no topic, no platform text:

```
[meeting 019f7abc-… — read: garyx meeting read 019f7abc-…]
```

UI "chat about this meeting" (slice 3) prefills the composer with that
line; the user may edit; the message travels existing paths unchanged;
the agent runs the CLI (whose own rendering is governed by 6.6).
Recognition of a prior reference is the cursor row. Discovery without a
reference: `garyx meeting list`. A deleted entity fails the command with
"entity not found"; stale lines are harmless history.

## 8. UI surfaces (slice 3)

- Desktop "Meetings" gallery (left rail): keyset-paged list (client-side
  live-first presentation), refresh on open + 30 s polling while visible
  (no SSE); actions: chat-about (prefill), abort (joining|live), delete
  (terminal). Platform text rendered as plain text.
- iOS: same catalog via gateway-scoped stale-while-refresh; route state
  and mapping in `GaryxMobileCore` with SwiftPM tests; packaged
  `xcodebuild` validation.
- No transcript/render_state work anywhere.

## 9. Configuration and deployment prerequisites

- `FeishuAccount.meeting_entities: bool` (`default_true`): disabling
  stops new admissions and unregisters the client; existing entities
  follow 5.1/5.2 rules (joining aborts at its deadline; live shows
  `no_client`; finalizing finalizes at its deadline).
- `GatewayConfig.meetings: MeetingConfig` — `poll_interval_secs` (30,
  clamp 10–120), `join_retry_window_secs` (300), `read_page_bytes`
  (65536).
- Developer console (blocks slice 2): subscribe the two `vc.bot.*`
  events; publish app version. Scopes already granted, live-verified.
- Public-repo hygiene: all fixtures sanitized/synthetic.

## 10. Failure and observability

Lifecycle transitions log info (entity, feishu id, CAS from→to, source).
Typed platform errors log with backoff and `failure_kind`/`failure_since`
state. `status_detail`, `end_source`, synthesized `stalled_reason` in
`garyx meeting list --json`. Boot repair logs truncated bytes/lines,
cursor corrections, cache-generation repairs, tombstone dispositions,
index rebuilds. `cache-repair` retries and `index_building` responses log
at debug.

## 11. Implementation impact map

| Area | Change | Nature |
|---|---|---|
| `garyx-models/src/local_paths.rs` | `default_meetings_dir()` | additive |
| `garyx-models/src/config.rs` | `FeishuAccount.meeting_entities`, `GatewayConfig.meetings` (serde-default) | additive |
| `garyx-channels/src/meeting_sink.rs` (new) | traits, `MeetingInvite`, typed errors, no-op impl | additive |
| `garyx-channels/src/feishu/types.rs` | 2 event structs | additive |
| `garyx-channels/src/feishu/ws.rs` | 2 dispatch branches → sink | additive |
| `garyx-channels/src/feishu/client.rs` | `bots_join`/`bots_events` + adapter | additive |
| `garyx-channels/src/plugin.rs`, `feishu.rs` | 1 constructor dependency | additive |
| `garyx-gateway/src/meetings/` (new) | service, coordinators, log writer/repair, locks, index, routes | new module |
| `garyx-gateway/src/garyx_db/mod.rs` | 2 tables + CRUD | additive |
| `garyx-gateway/src/route_graph.rs` | 7 routes | additive |
| `garyx-gateway/src/composition/*` | service wiring + sink injection | additive |
| `garyx/src/commands/meeting.rs`, `cli.rs`, `main.rs` | CLI subcommand | additive |
| Desktop / iOS (slice 3) | gallery + prefill action | additive |

**Not touched:** bridge, providers, router dispatch, transcripts,
render_state, SSE, thread lifecycle, prompt assembly, queued-input
paths.

## 12. Test plan (complete matrix)

**Storage/page commits:** crash after `seg` lines before `ckpt` (whole
page truncated at boot, re-pulled — no dupes, no loss); crash after
`ckpt` before SQLite (cache-repair, forward-only, no re-pull); delayed
repair vs newer commit (generation guard blocks regression: P1-fail →
P2-success → P1-delayed-repair); cache-fail → immediate finalize
attempt (terminal barrier forces cache+index success first); in-flight
page timeout consumes no cursor; `has_more` immediate rescheduling;
empty-poll ckpt; torn-line truncation; escape-inflation splits; crash
after each split chunk with no ckpt; `fdatasync` ordering fault
injection; cache rebuild equals log.

**Record bounds:** property tests generated from the normative caps of
4.2 (max speaker, 64-byte/hashed source ids, max member counts, share
title/URL caps) prove no encoded line exceeds 32 KiB; oversized source-id
hashing is deterministic; merge-only-if-fits (a merge that would split is
not merged); UTF-8-boundary truncation for every truncated field;
continuation source mapping uniqueness.

**Ticks/scheduling:** hanging `poll_events` cut by timeout; sustained
`has_more` commits page-by-page with content readable between commits;
command executed immediately when idle (joining/stalled admin abort) vs
after the current page commit.

**Lifecycle:** end>abort arbitration; GraceExpired live/finalizing
(accelerates); 10005 live (abort) / finalizing (complete early);
AbortRequest refused in finalizing; abort intent crash-resume; drain
runs to deadline (no quiescence: transcript arriving after two empty
pulls before the deadline is captured); joining deadline while stalled
aborts; finalizing stalled finalizes at deadline; join succeeding during
grace; end-during-joining via join-then-first-poll fixtures; duplicate
invites (same event id; distinct event ids) collapse; reinvite after
terminal and after delete creates fresh entities (S2); admission insert
failure path (S1); restarts in every non-terminal state resume.

**Registry/failure (R7-05):** transport stalled 14 min → auth failure →
clock resets and reason switches (no 16-min auth claim); auth →
unregister → register → auth (clock does not span the gap); register
success clears synthesized `no_client` immediately (derived, not
stored); register/replace nudges stalled entities without timers;
disable→enable; per-poll registry fetch observes replaced clients;
finalizing with no client finalizes at deadline; `(failure_kind,
failure_since)` pair CHECK.

**Deletion:** all crash points (rename/commit/remove); missing-dir empty
entity; tombstone-before-repair boot ordering; delete waits for or
cancels index rebuild (no orphan dir recreated); confirm/read on deleted
→ 404; cascade removes cursors.

**Fetch/confirm:** concurrent first fetch (one row, shared receipt);
concurrent claim on existing row (one winner; loser re-serves winner's
span); fetch/confirm/delete tripartite race; crash-before-flush vs
confirm-committed-response-lost (distinct outcomes asserted); stale
receipt no-op; cursors never regress; empty increment creates the row;
`--full`/`--range` never create rows; reader-id byte-length CHECK
(multi-byte chars measured as bytes: two emoji ≠ 2 bytes); same reader
id from two callers shares a cursor (S6/S7 fixtures); many minted reader
ids (S7 documented behavior + cascade cleanup).

**Budget (R7-02, RR8-01):** `--max-bytes 4096` with a 32 KiB line →
single segment served with the overshoot note; newline-heavy segment
whose framed rendering exceeds raw JSON size; maximal topic/header
metadata counted before content; floor rejection (0/1/4095); no
zero-progress token in any case; **pending-replay indivisibility**:
64 KiB/JSON claim → response lost → 4 KiB/human retry re-serves the
entire span with the replay note; concurrent winner/loser with different
budgets both deliver the winner's full span; new claims capped by
`read_page_bytes` regardless of requested budget.

**Snapshots/index:** `--full` across pages pinned to one snapshot under
concurrent appends; sliding renewal across >10 min total stream;
resume <10 min inactivity works; >10 min restarts with clear message;
token shell-safety; index written inside the terminal barrier (crash
before terminal CAS → intent stage rewrites); torn/stale/version/crc
mismatched index discarded and rebuilt once; single-flight rebuild under
concurrent cold reads; client-timeout-then-retry hits the completed
index; `index_building` retryable response; cold high-seq read on a long
terminal log within budget.

**List:** weak-insert semantics demonstrated (same-timestamp smaller id
appears later; clock-skew insert); unreturned-row deletion; updates move
nothing.

**Trust/framing:** hostile topic/speaker/transcript with
header-lookalikes, ANSI/OSC, Bidi controls, C1 bytes, **U+2028/U+2029
(alone and mixed with CR/LF/ANSI/Bidi)** — stripped/normalized/framed in
human output (no unprefixed line from platform text), string-encoded in
`--json`; S6 elicitation fixture recorded as documented behavior.

**Epoch/loss (RR8-02, RR9-01, RR9-04):** DB generation 7 + missing log →
epoch bump + counter reset → new pages commit under the new epoch →
restart → immediate finalize (barrier verifies read-back, not zero-row
success); non-empty terminal log missing → `content_state='lost'` error,
never stale totals, and the aborted entity's original `status_detail`
survives; concurrent first detection is idempotent under the write lock;
detection-vs-delete race → 404 wins; empty-log abort persists a valid
empty index; stale-epoch delayed repair rejected. **Read-domain
invalidation:** rollover with existing confirmed=100 → next read starts
at new-epoch seq 1 (no skip); rollover with existing pending + old
receipt → pending cleared, old receipt confirm fails with content-loss
error; old snapshot token → `snapshot invalidated by content loss`; old
`--range` epoch → same error; mixed-epoch ckpt lines treated as invalid
tail; loser-refresh: snapshot A (latest=10) → append to 20 → B claims
1..20 → A re-preflights until its snapshot covers 20, then re-serves;
`--full/--range --max-bytes 1GiB` still served in `read_page_bytes`
pages server-side.

**Recognition preflight (RR8-03):** first cold terminal read hitting
`index_building` creates no cursor row; successful retry creates it;
lost-response-after-pending keeps the row (distinct from failed first
read).

**Stalled scoping (RR8-05):** terminal + unregistered account shows no
`no_client`; auth-failed finalizing → finalized clears the failure pair;
joining/finalizing/aborting statuses render correctly in read/list
output.

**WS path (S1):** fixture records which path carries invite/ended;
raw-text carriage asserts the slice-2 halt condition.

Scope: `cargo test --all-targets` for `garyx-channels`, `garyx-gateway`,
`garyx-models`, `garyx`; tier1 fast loop; slice 3 adds SwiftPM +
`xcodebuild` + packaged desktop check.

## 13. Delivery slices

1. **Entity core + read protocol** (no Feishu dependency): DDL, log
   writer/repair, locks + index, fetch/confirm + snapshots, budget
   handling, CLI list/read/delete, fixture tests. Gate: read-protocol,
   storage-fault, deletion, concurrency, record-bounds, and framing
   suites green; hand-seeded entity readable incrementally from two
   reader ids with independent receipt-confirmed cursors including
   forced-failure re-serves.
2. **Ingestion:** sample capture gate (3.1) → event structs, sink +
   registry lifecycle, coordinator with intent CAS + barriers,
   page-atomic polling, grace drain, recovery, admin abort, console
   subscriptions. Gate: real meeting live→finalizing→finalized with
   correct segments; restarts in every non-terminal state resume;
   lifecycle + registry/failure suites green.
3. **Experience:** galleries + prefill chat + polling refresh. Gate:
   card → prefilled message → agent reads increment → answer cites new
   segments, on packaged desktop + simulator.

## 14. Open questions (defaults chosen, non-blocking)

- Retention: manual deletion only.
- One entity per admission (S2).
- `meeting_activity_v1` ignored in Phase 1.
- Cursor quota/GC and any real authorization model are future work gated
  on S6/S7 being revisited.
- Per-turn automatic context injection: explicitly out of scope; a
  future separate design if ever wanted.
