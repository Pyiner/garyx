# Feishu meeting entity

Status: revision 21 — sample-gate amendment (Section 3.1 triggered):
production-grade payload evidence from mino_server replaced the polling
assumption with push-based capture. Reviewed as part of the slice-2
implementation gate. Prior baseline: revision 20, adversarial review
#TASK-2337 rounds 1–19, 100% PASS.
Author: gary (design)
Scope ruling (user, 2026-07-16): orthogonal side-system; existing runtime
flows are not modified; agents read via CLI; conversation references are
plain text; read cursors live on the read side.
Supersedes: `docs/design/feishu-group-listen-mode.md` (group listening
cancelled). The subsystem never automatically copies or injects the
canonical meeting log into thread ledgers; ordinary user, tool-result,
and assistant turns may of course quote meeting content through existing
flows — that is normal conversation, not a subsystem write.

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

1. Meetings only. 2. Invite-driven join only (the manual debug-join CLI was cut in review round 11: it cannot map onto the invite-keyed admission contract without synthetic-event machinery that Phase 1 does not need; slice-2 testing uses real invites against a scratch meeting).
3. Live near-real-time accumulation (~30 s platform latency). 4. CLI-only
reads, per-reader cursors, incremental default, self-describing. 5. End
signal freezes the entity. 6. The cursor row is the recognition of prior
reference. 7. Orthogonality: additive touchpoints only (Section 11).

Product sign-off items (explicit, owner-revocable):

- **S1** On the Feishu WS **protobuf data-event path**, an ACK send is
  **attempted** before processing; a send failure is logged and
  processing continues. Meeting invite/ended events **bypass the
  generic 30-minute in-memory event dedup cache** (their idempotence is
  owned by the durable `meeting_invite_keys` table and end-path CAS,
  which absorb platform redelivery correctly even after an admission
  failure — the in-memory cache would wrongly swallow a redelivery that
  arrives after a failed admission insert). The loss rule is stated on
  the only durable fact: **an invite is admitted iff its insert
  committed.** If the insert did not commit — whatever the cause:
  process exit before/during admission (with the local ACK outcome
  being success, error, or unknown — the send may be in flight at
  crash), or 3 exhausted retries (100 ms / 1 s backoff, error-logged
  when the process survives to log it) — the invite is recovered only
  if the platform later redelivers (a courtesy, likelier when our ACK
  did not reach it, never guaranteed) and that redelivery's insert
  commits; otherwise it is lost, possibly without any local log
  (nothing durable exists to log from). Redelivery after a committed
  insert is a unique-key no-op. The raw-text
  fallback path has no equivalent ACK evidence; the sample gate (3.1)
  pins which path carries invite/ended events; if the raw-text path can
  carry them, slice 2 halts for a design amendment.
- **S2** One entity per admission: re-invites after terminal state or
  deletion create new entities capturing from that point. **Physical
  deletion also deletes the admission idempotency key** (RR12-03): a
  platform redelivery of the *same old* invite event arriving after the
  entity was deleted will be admitted as a fresh entity — accepted (the
  realistic window is minutes; manual deletion of a meeting whose stale
  invite is still in flight is an owner action on their own data) — the
  `meeting_invite_keys` rows cascade-delete with the entity by design,
  and no independent tombstone-key store is kept. Four leave-related residual risks are likewise
  accepted (RR14-04, RR15-04): ① a **joining** abort never attempts
  leave (no meeting id exists locally), so a late remote join can leave
  the bot in the meeting without capture until removed manually; ② a
  **no-client** live abort terminates locally only; ③ a crash between
  the abort CAS and the single leave attempt skips it; ④ a
  registered-client leave attempt may itself **time out (20 s) or
  fail**, and is never retried. In all four the meeting owner can
  always remove the bot manually.
- **S3** The final ~30 s of speech becomes readable during the grace
  drain, not instantly.
- **S4** (push model, rev21) Capture is at-most-once: an activity event
  that arrives while the process is down, or crashes before its batch
  commit, is **permanently lost** — there is no replay channel in
  Phase 1 (the platform's pull API exists but is unadopted; a future
  gap-fill fetch may reduce this window). A gateway restart mid-meeting
  therefore loses the activity pushed during the outage — the same
  acceptance as the bot not having been present for that stretch.
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
| B1 | Feishu WS protobuf data-event path **attempts** an ACK send before processing (a failed send is logged and processing continues); a raw-text fallback path processes without that ACK evidence. Only `im.message.receive_v1` is handled today; other event types fall to an ignore branch; 30 min in-memory event-id dedup | `garyx-channels/src/feishu/ws.rs:914,925,997,1049-1072`, `feishu.rs:95` (line refs approximate — all B-row refs re-pinned against HEAD at implementation start) |
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
- `POST /open-apis/vc/v1/bots/leave`: leave by long meeting id (used
  only as best-effort abort compensation, 5.2).
- **In-meeting content arrives by push, not pull** (sample gate,
  3.1): `vc.bot.meeting_activity_v1` events carry
  `meeting_activity_items[]`, each with `activity_event_type`
  (`transcript_received` | `chat_received` | `participant_left`),
  `meeting.id`, and a per-type item array. Production evidence:
  mino_server (`biz/domain/dmservice/meetingbotservice/service.go`,
  `service_test.go`, `trigger_event.go` — exact field names below). A
  pull API (`GET /open-apis/vc/v1/bots/events`) also exists (lark-cli
  uses it) but is not used in Phase 1 (no production payload evidence;
  a future gap-fill fetch after restarts may adopt it).
- Pinned payload shapes (verbatim from mino_server tests):
  - invite `event`: `meeting.id` (a pre-join reference — the real
    meeting id is the one `bots/join` returns), `meeting.meeting_no`,
    `meeting.topic`, `bot.id`, `inviter.id`. **No `call_id` exists.**
  - ended `event`: `meeting.id` only.
  - activity transcript item: `text`, `language`, `sentence_id`,
    `start_time_ms`, `end_time_ms`, `speaker.id.open_id`,
    `speaker.user_name`.
  - activity chat item: `message_id`, `content`, `sent_timestamp`,
    sender under `operator.id.open_id` / `operator.user_name` (fallback
    `operator.name`).
  - activity participant_left item: `participant.id.open_id`.
  - JSON numbers in these payloads can be bare integers (mino hit
    float64 precision loss via sonic); parse ids as strings or
    arbitrary-precision, never via f64.
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

**Gate status: satisfied by production evidence (2026-07-16).** The
payload shapes above are pinned from mino_server's production
implementation and its committed test fixtures (not from live capture):
`biz/infrastructure/channel/lark/trigger_event_vc_bot_test.go`
(invite/ended), `biz/domain/dmservice/meetingbotservice/service_test.go`
(activity items), `biz/infrastructure/channel/lark/vc_client.go`
(join request/response). Remaining slice-2 verification (a check, not a
blocking gate): the first real invited meeting must confirm these shapes
against Garyx's own WS delivery — a mismatch stops rollout for an
amendment. Facts still unpinned and accepted as implementation-time
checks: whether `vc.bot.*` events arrive on Garyx's long-connection
protobuf path (mino uses webhooks; Garyx console subscribes in
long-connection mode — first real event settles it), and duplicate-join
behavior (mino re-joins unconditionally with no error-code branch,
suggesting benign semantics; our retry logic treats a join error that
carries the meeting identity as success-equivalent).

## 4. Entity model and storage

### 4.1 SQLite DDL (normative)

```sql
CREATE TABLE IF NOT EXISTS meetings (
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
  -- pairing enforced below with: CHECK ((content_state = 'lost') = (content_lost_at IS NOT NULL))
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
  CHECK ((failure_kind = '') = (failure_since IS NULL)),
  CHECK ((content_state = 'lost') = (content_lost_at IS NOT NULL))
) STRICT;
CREATE TABLE IF NOT EXISTS meeting_invite_keys (
  invite_event_id TEXT NOT NULL PRIMARY KEY,
  meeting_id      TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  observed_at     TEXT NOT NULL
) STRICT;
-- meetings.invite_event_id remains as "first admitting event" provenance;
-- durable admission idempotency is owned by meeting_invite_keys (RR14-02).
CREATE UNIQUE INDEX IF NOT EXISTS idx_meetings_active_no
  ON meetings(account_id, meeting_no)
  WHERE status IN ('joining','live','finalizing','aborting');
CREATE UNIQUE INDEX IF NOT EXISTS idx_meetings_active_fid
  ON meetings(account_id, feishu_meeting_id)
  WHERE feishu_meeting_id <> ''
    AND status IN ('joining','live','finalizing','aborting');
CREATE INDEX IF NOT EXISTS idx_meetings_created ON meetings(created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_meetings_status  ON meetings(status);

CREATE TABLE IF NOT EXISTS meeting_read_cursors (
  meeting_id    TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  reader_id     TEXT NOT NULL
    CHECK (length(CAST(reader_id AS BLOB)) BETWEEN 1 AND 128),  -- bytes, not chars
  log_epoch     INTEGER NOT NULL CHECK (log_epoch >= 0),  -- no default: always explicitly inserted from meetings.log_epoch (RR10-01)
  confirmed_seq INTEGER NOT NULL DEFAULT 0 CHECK (confirmed_seq >= 0),
  pending_from  INTEGER,
  pending_to    INTEGER,
  receipt       TEXT,
  updated_at    TEXT NOT NULL,
  PRIMARY KEY (meeting_id, reader_id),
  CHECK ((pending_from IS NULL) = (pending_to IS NULL)
     AND (pending_from IS NULL) = (receipt IS NULL)
     AND (pending_from IS NULL OR
          (pending_from > confirmed_seq AND pending_to >= pending_from)))
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
table: `epoch` is a non-negative 64-bit integer (starts at 0,
incremented once per content-loss rollover).

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
| `epoch` (ckpt lines, tokens, spans) | non-negative 64-bit integer (`0..=i64::MAX`), starts at 0, +1 per content-loss rollover |
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

**Batch-atomic commit (the durability core; push model, rev21):** the
commit unit is one received `vc.bot.meeting_activity_v1` event (one
bounded batch of items). On delivery the coordinator commits it
atomically under the entity I/O write lock:

1. normalize the batch → `seg` lines (bounded as above; items whose
   `sentence_id`/`message_id` already appear in the log are dropped —
   redelivery dedup);
2. append `seg` lines → append one `ckpt {event_id = the WS event's id}`
   → `fdatasync`;
3. SQLite cache transaction guarded by **epoch + monotonic generation**
   (R7-04, RR8-02): `UPDATE meetings SET closed_segment_count=…,
   byte_size=…, cache_generation=:gen, updated_at=… WHERE id=:id AND
   log_epoch = :epoch AND cache_generation < :gen` — `:gen` is the
   checkpoint ordinal within the current epoch. (`poll_cursor` is
   retired; the ckpt's `event_id` chain is the batch ledger.)

Once the `ckpt` is fsynced the batch is committed; a failed cache
transaction puts the entity in retryable `cache-repair` (backoff; each
repair re-derives from the canonical log under the I/O lock with the
generation guard); recovery is forward-only. There is no polling loop,
no page token, no tick budget: data arrives when the platform pushes
it.

**Boot repair** (non-terminal entities only; terminal logs are never
scanned at boot): validate line by line; truncate everything after the
last valid `ckpt` (torn and complete `seg` lines alike — they belong to
an uncommitted batch, which is simply lost per S4; there is nothing to
re-pull in the push model); rebuild SQLite caches (with the generation
guard) and the in-memory offset index in the same pass. Missing
directories/files are tolerated everywhere (S8).

### 4.3 Entity deletion

Terminal entities only (409 otherwise). Under the entity I/O write lock:

1. wait for / cancel any in-flight index rebuild for the entity (R7-04);
2. atomic `rename {id}/ → {id}.tombstone/` — a missing dir is a legal
   empty entity (a joining-deadline abort may never have appended):
   skip the rename;
3. DB transaction deletes the `meetings` row (cascades cursor rows and
   invite-key rows);
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
    async fn join(&self, meeting_no: &str, password: Option<&str>)
        -> Result<JoinedMeeting, MeetingApiError>;
        // POST /vc/v1/bots/join {join_identify:{meeting_no}, join_type:1,
        // password?} -> data.meeting.id (rev21: no call_id — it does not
        // exist in the real payload)
    async fn leave(&self, feishu_meeting_id: &str)
        -> Result<(), MeetingApiError>;           // best-effort abort compensation
}
pub trait MeetingEventSink: Send + Sync {
    fn register_client(&self, account_id: &str, client: Arc<dyn MeetingPlatformClient>);
    fn unregister_client(&self, account_id: &str);
    fn on_meeting_invited(&self, invite: MeetingInvite);
    fn on_meeting_activity(&self, account_id: &str, event_id: &str, payload: serde_json::Value);
        // rev21: bounded enqueue of the raw activity event; the
        // coordinator normalizes and commits (4.2)
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

**Admission → joining (one atomic transaction, RR14-02):**
① if `invite_event_id` exists in `meeting_invite_keys` → no-op (exact
redelivery); ② else find an active entity for
`(account_id, meeting_no)` — found → insert a key row linking this
event id to it (a *distinct* event for an already-active meeting is
thereby durably recorded; its redelivery stays a no-op even after the
entity later reaches a terminal state); ③ else create the entity
(`status='joining'`, `join_deadline_at = now + join_retry_window`,
absolute) and its key row. Entity deletion cascades its key rows,
preserving S2's delete-resets-idempotency semantics. Content dir is created lazily by the
writer (`ensure_dir` before first append). Join attempts run inside the
coordinator's `tokio::select!` alongside the command queue and the
absolute deadline (RR11-03): attempts are paced at 20 s intervals
(RR12-02: an immediately failing attempt waits out the remainder of its
20 s slot — no hot loop), each carries a timeout of `min(20 s, remaining
window)`, and a deadline expiry, `AbortRequest`, `Shutdown`, or registry
replacement wins the select and cancels the in-flight join **future**.

**Remote join is a side-effecting request whose cancellation is local
only** (RR12-02): the platform may complete the join after our timeout,
cancellation, or crash. The design handles the unknown-outcome window
as follows:

- A timed-out/cancelled/crash-interrupted attempt leaves the entity in
  `joining`; the next attempt (or boot recovery) simply calls `join`
  again. The sample gate (3.1) **pins duplicate-join behavior**: the
  expected platform semantics (re-joining an already-joined meeting
  returns the same `meeting.id` idempotently) must be confirmed from
  captured fixtures; if the platform instead errors on duplicate join,
  the mapped error is treated as success-equivalent iff it carries the
  meeting identity, else slice 2 halts for an amendment.
- **Leave compensation applies only to `live → aborting`** with a
  registered client and a non-empty `feishu_meeting_id`: one
  best-effort `bots/leave` under a **20 s timeout** (RR14-04 — timeout
  or failure is logged and the barrier→aborted path continues
  immediately; exactly one attempt, never retried; no persisted marker
  — a crash between abort CAS and the leave attempt simply skips it; a
  hanging platform call cannot hold the entity in `aborting`). It is **not** attempted from `joining` aborts: at that point no
  meeting id exists locally (the join response was cancelled or never
  arrived), so there is nothing to leave with. The complete accepted
  residual-risk list lives **only** in S2 (four items — joining late
  join, no-client abort, crash-skipped leave, leave timeout/failure);
  this section intentionally does not duplicate it.

On success — CAS `joining→live`, backfill `feishu_meeting_id`
(normalized) and `topic` (normalized per 4.1 bounds); on deadline →
abort path. The deadline applies **even while stalled** (it models invite
validity). Join succeeding during the grace window is legal; polls then
return data until `GraceExpired` finalizes.

**Live capture (push, rev21):** activity batches commit per 4.2 as they
arrive; between events the coordinator is idle (no polling loop). End
detection is dual, mirroring production evidence: ① the
`vc.bot.meeting_ended_v1` push, ② an activity `participant_left` item
whose `participant.id.open_id` equals the bot's own open id (leave or
removal — both arrive this way). `NotInMeeting` from a join/leave call
retains its abort semantics.

**Abort HTTP protocol (RR15-03, RR16-02, R17-02) — normative
state/response table for `POST /api/meetings/{id}/abort`:**

| Entity state at linearization | Response |
|---|---|
| `joining`, `live` | command admitted to the coordinator; durable intent CAS to `aborting` commits; then `200 {status:"aborting"}` |
| `aborting`, `aborted` | `200` idempotent no-op — served by a **handler-level DB fast path** (a point read of `status`, never enqueued to the coordinator), so a retry arriving while a 20 s leave attempt is in flight still returns immediately |
| `finalizing`, `finalized` | `409 abort_refused_finalizing`; a queued abort that loses end>abort arbitration at the page boundary receives the same `409` |
| deleted | `404` |

Ordering and ownership rules (R18-01 — one serialization domain, no
TOCTOU):

- All abort requests for an entity pass through a **single per-entity
  abort domain** that owns the operation's **entire lifecycle**
  (R19-01): (a) status check, (b) operation creation **including the
  successful enqueue of its one `AbortRequest` command** — the
  operation becomes visible to later requests only after the enqueue
  succeeded; an enqueue failure (queue closed, coordinator gone)
  atomically rolls the operation back and completes any waiters with a
  typed retryable error, (c) waiter registration — later requests
  either join a **not-yet-completed** operation or read the completed
  result / durable status; there is no window in which an orphan
  operation can accumulate waiters with no command behind it, and no
  bare point-read-then-enqueue path.
- **Result publication also runs through the domain**: the coordinator
  completes the operation by, inside the domain, marking it
  `completed(outcome)` and then draining its waiters — so a request
  racing the completion either registered before (gets the drain) or
  enters after (sees `completed`/the new durable status; it can never
  miss the notification or spawn a second command). Outcomes are
  answered **by actual result**: abort won → `200 {status:"aborting"}`;
  end won the page-boundary arbitration → `409
  abort_refused_finalizing`; entity deleted meanwhile → `404`.
  Answering precedes the (up to 20 s) leave attempt. A successfully
  enqueued command is owned by the service; HTTP disconnects cancel
  nothing after visibility, and before visibility the domain's
  rollback rule above applies.
- Timing, honestly: admission may wait for the current page commit —
  fetch is bounded at 10 s but the durability phase (`fdatasync` +
  SQLite with up to 5 s busy wait) is additional — so a first request
  may exceed the shared 10 s POST budget. `garyx meeting abort`
  performs **one automatic retry on timeout only** (typed transport
  cause, §6.6): the retry re-enters the abort domain, where it either
  joins the still-in-flight operation (answered at its outcome) or
  hits the status fast path — in both cases without waiting behind the
  leave attempt. Its result follows the table (200/409/404), and a
  second timeout is reported to the caller.

### 5.3 End path and grace drain

On the end signal (either path above): CAS `live→finalizing`,
`ended_at=now`, `grace_deadline_at = now + 4 min`, `end_source`
recorded (`push` or `participant_left`). **Finalizing in the push model
means staying subscribed for trailing activity** — late transcript
batches keep arriving for a short while (production evidence: mino
keeps a 6 h tombstone to absorb them; our capture window is 4 min,
after which the terminal barrier runs and trailing pushes for a
finalized entity are dropped with a debug log). `AbortRequest` during
finalizing is refused; a stalled finalizing entity still finalizes at
its deadline. Unknown-entity end signals and activity for unknown/
terminal entities are logged and dropped. Restart during finalizing
resumes the countdown to the persisted deadline.

### 5.4 State machine

```
 invite ─admission insert─> JOINING ─join ok─> LIVE ─batch commits (push)─┐
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
- `garyx meeting read <id> [--full | --range A..B [--epoch E] |
  --continue <token>] [--thread <id>] [--json] [--max-bytes N]` —
  `--epoch` is valid only with `--range` and defaults to the entity's
  current epoch (non-current → content-loss error; old spans are never
  silently re-mapped). **`--continue <token>` is a token-only mode**
  (RR15-02): mutually exclusive with `--full`, `--range`, and
  `--epoch`; the token envelope carries the mode, position, epoch, and
  range bounds, the CLI sends `{mode: <from token>, continue_token}`
  verbatim, and the server validates the token against the path entity
  and its own mode. Mismatches (token for another entity, token mode vs
  body mode, expired) are named 400s. Headers print the exact resume
  command: `garyx meeting read <id> --continue <token>`.
- `garyx meeting abort <id>` (admin; initiates from joining|live;
  idempotent `200` on aborting/aborted; refused `409` on
  finalizing/finalized; one automatic retry on timeout — full table and
  ordering rules in 5.2)
- `garyx meeting delete <id>` (terminal only)

API:

```
GET    /api/meetings?limit&page_token      -> keyset-paged list
GET    /api/meetings/{id}                  -> metadata
POST   /api/meetings/{id}/read             -> fetch (wire schema below)
POST   /api/meetings/{id}/read/confirm     -> confirm {reader_id, receipt, log_epoch}
POST   /api/meetings/{id}/abort            -> admin abort
DELETE /api/meetings/{id}                  -> delete
```

**Read wire schema (RR14-03):**

```
{
  mode:           "incremental" | "full" | "range",
  reader_id?:     string,        // required iff mode=incremental
  range_start?:   int, range_end?: int,  // required iff mode=range; closed interval [start, end]
  epoch?:         int,           // valid iff mode=range; default current
  continue_token?: string,       // valid iff mode=full|range; mutually exclusive
                                 // with range_start/range_end/epoch (token carries them)
  max_bytes?:     int            // floor 4096
}
```

Field-combination violations are 400s named after the offending field.
**Responses are always structured JSON** — `{meta, segments[]}` where
`meta` carries mode, span, totals, status, epoch, receipt/continuation,
and notes; budgets are measured on this serialized JSON response
(single, format-independent budget algebra — claim sizing no longer
depends on presentation). The CLI's human format is a **pure local
presentation layer** over the structured response: the `│ ` framing,
control-character stripping, and U+2028/2029 normalization of 6.7 are
CLI rendering rules; `--json` mode prints each page's structured
response as one NDJSON line. A pending span claimed under any format
re-serves identically under any other (spans are segment ranges;
format was never part of the claim).

Incremental (default) requires a reader identity (`GARYX_THREAD_ID` env
or `--thread`; missing both → error naming both remedies). `--full` and
`--range` are stateless paged snapshot peeks: no identity, no cursor rows
ever created or touched. Only a successful incremental fetch creates a
cursor row.

**Response budget (R7-02, RR8-01, RR15-01, RR16-01):** the **single
normative budget algebra is the serialized byte length of the
structured JSON response body**, measured on the exact HTTP body bytes
sent. Success DTO (complete; `?` = nullable):

```
meta: {
  mode: string, entity_id: string, log_epoch: int,
  status: string, status_detail: string, end_source: string,
  stalled_reason: string, content_state: string,
  topic: string,                       // normalized per 4.1 bounds
  started_at: string, ended_at: string?, finalized_at: string?,
  content_lost_at: string?, updated_at: string,
  span_from: int?, span_to: int?,      // null for empty increments
  closed_total: int,
  receipt: string?,                    // incremental only
  continue_token: string?,             // stateless, more pages
  notes: [string]                      // e.g. overshoot / pending-replay notes
}
segments: [ … ]   // isomorphic to the canonical `seg` line schema of 4.2
                  // (same fields, same types, same bounds), minus the "t" tag
```

There is **no render-mode input anywhere on the server**: claims,
budgets, and replay are functions of the JSON body alone; human
formatting is CLI-local presentation whose stdout may legitimately
exceed the JSON budget. The single-segment minimum-progress exception
is defined on the same algebra: a response whose serialized JSON —
containing exactly one segment plus `meta` — exceeds the budget is
still served, with an explanatory entry in `meta.notes`. `--max-bytes`
(floor 4096; below → CLI error) is the client's requested value for
this JSON budget. **Multi-page accumulation** (stateless modes,
R17-01): the CLI counts consumed bytes as the **exact HTTP response
body lengths** (the shared JSON helper is extended to surface the raw
body length alongside the parsed value — an additive change to
`gateway_client.rs`, listed in §11); each subsequent request sends
`max_bytes = requested_total.saturating_sub(consumed)`; when the
remainder falls below 4096 the CLI stops and prints the resume command
(no sub-floor request is ever sent). **Any page's first segment may
push the final cumulative total past `requested_total`** — the
single-segment minimum-progress exception applies per page, not only
to the first page; the requested total is a target, and each overshoot
is named in that page's `meta.notes`. Two rules govern the budget's
interaction with pending spans:

- **Every newly produced page is budget-bounded server-side:** the
  claimed span (incremental) and every `--full`/`--range` snapshot page
  are sized to `min(requested_max, read_page_bytes)` — `read_page_bytes`
  is the **server-side hard cap** on any newly produced page (RR9-03: a
  huge `--max-bytes` cannot bypass server pagination), so no caller can
  mint an arbitrarily large pending span or response — with the single-segment
  minimum-progress exception (a lone segment whose single-segment JSON response exceeds the budget
  is still claimed and served, with a `meta.notes` entry).
- **Pending replay is indivisible:** an existing pending span is the
  atomic delivery unit. A re-serve returns the **entire pending span**
  regardless of the current request's budget (`meta.notes` carries
  `pending replay exceeds requested budget` when applicable), because
  serving a subset while confirming the original receipt would silently
  skip the remainder. Budgets apply to *new* claims only.

A zero-progress response or non-advancing continuation token is never
returned. (There is no separate metadata budget: `meta` is part of the
same serialized JSON body the single algebra measures.)

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
  `(log_epoch, closed_latest, log_byte_offset)`; slicing never scans
  past the snapshot offset, and every downstream span, token, and
  response carries the snapshot's epoch. Terminal entities take the
  lock directly; live entities' snapshots are requested through the
  coordinator.
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
   **The entity read lock is held from preflight through step 3's
   response assembly** (RR11-01), so a rollover (which takes the write
   lock) cannot interleave between preflight and any cursor write; the
   epoch triple-check below is defense in depth against lock bugs, not
   the primary guard.
   Before serving **any** branch — pending re-serve, empty span, or new
   claim — the transaction verifies
   `meetings.log_epoch == cursor.log_epoch == snapshot_epoch`; any
   mismatch returns the content-loss error (the caller re-runs
   preflight).
1. **Ensure row (epoch-guarded):**
   `INSERT INTO meeting_read_cursors (meeting_id, reader_id, log_epoch,
   confirmed_seq, updated_at)
   SELECT :m, :rd, log_epoch, 0, :now FROM meetings
   WHERE id = :m AND log_epoch = :snapshot_epoch
   ON CONFLICT DO NOTHING` — the insert derives the epoch from the
   entity row **and** requires it to still equal the snapshot epoch;
   zero rows inserted with no existing cursor row means the rollover
   raced (content-loss error). Created on the first incremental fetch
   that passes preflight, even if the increment is empty (the row is
   the recognition state).
2. **Fetch:** read the row. Pending exists → re-serve exactly
   `(pending_from..pending_to)` with the stored receipt (at-least-once;
   receipts are shared, not per-caller). Else compute the next span
   `(confirmed_seq, min(latest_snapshot, budget-limited end))` and claim:
   `UPDATE … SET pending_from=:f, pending_to=:t, receipt=:r, updated_at=now
   WHERE meeting_id=:m AND reader_id=:rd AND log_epoch=:snapshot_epoch
   AND confirmed_seq=:expected AND pending_from IS NULL`. Zero rows are
   **trichotomized** (RR10-01): row gone → entity deleted (404); row's
   epoch ≠ snapshot epoch → rollover raced this request → return the
   content-loss error (the caller re-runs preflight); else a concurrent
   fetch won →
   re-read the row and **re-run preflight until the snapshot covers the
   winner's `pending_to`** (RR9-02: the loser's earlier snapshot may
   predate the winner's claim; slicing never exceeds the snapshot, so
   the loser refreshes rather than violating it), then re-serve the
   winner's span and receipt. An empty span
   serves the empty-increment header without claiming pending.
3. **Confirm** (`/read/confirm {reader_id, receipt, log_epoch}` — the
   CLI passes the epoch it received with the fetch):
   `UPDATE … SET confirmed_seq=pending_to, pending_from=NULL,
   pending_to=NULL, receipt=NULL, updated_at=now
   WHERE meeting_id=:m AND reader_id=:rd AND receipt=:receipt
   AND log_epoch=:request_epoch`. Outcome discrimination (RR10-01):
   request epoch ≠ entity's current epoch → **content-loss error**
   (rollover invalidated the span); request epoch current but receipt
   unknown/cleared → **idempotent no-op success** (already confirmed);
   deleted entity → 404.
4. **The CLI performs fetch → render → stdout flush → confirm in one
   invocation.** Crash/broken pipe before flush → pending survives → the
   same span re-serves next time. Confirm committed but its response
   lost → pending already cleared → the next read serves the next span
   (safe: content was flushed before confirm was attempted). Cursors
   never regress; row CAS serializes concurrent readers.

### 6.5 Stateless snapshots (`--full`, `--range`)

The first response pins `(log_epoch, closed_latest, log_offset)`; every
page returns
a **fresh token for the same snapshot** with a sliding 10-minute
inactivity expiry. Tokens are base64url (shell-safe) encoding
`{entity_id, log_epoch, snapshot, next_seq, mode, origin_range_start,
range_end, checksum, issued_at}` (R17-03: `origin_range_start`
preserves the original request's start so an expiry can name the exact
restart command) — a token whose epoch is no longer current fails with
`snapshot invalidated by content loss`.

**Typed read errors (R17-03):** `/read` errors use a structured
envelope `{error: {code, message, restart_command?}}` — codes include
`token_expired`, `snapshot_invalidated_by_content_loss`,
`index_building`, `content_lost`, and the named 400s of 6.1. The CLI
parses this envelope from the raw response body (the generic helper's
flattened `Rejected` text is insufficient for branching; the meeting
CLI reads the raw body — additive, §11). `restart_command` for
`token_expired` is server-built from the validated token
(`garyx meeting read <id> --range <origin_range_start>..<range_end>
--epoch <E>` or `--full`), emitted only after checksum/path/mode
validation succeeds; the CLI prints it and exits nonzero — never an
automatic restart.
The CLI loops within one invocation streaming pages to stdout until
exhausted, or stops once **cumulative fetched JSON bytes** reach
`--max-bytes` (the same single algebra; human stdout size does not
gate) and prints the resume command with the latest token. Inactivity >10 min → the server returns a typed `token_expired` 400;
the CLI does **not** auto-restart — it prints the original full/range
command for the user/agent to re-run explicitly (predictability over
convenience; unified with the named-400 rule of 6.1). Appends beyond the
snapshot are invisible to that snapshot's pages.

### 6.6 Transport seam in the CLI (R18-02)

`gateway_client.rs` gains one shared low-level primitive (additive; the
existing JSON helpers become wrappers over it):

```
struct RawGatewayResponse { status: u16, raw_body: Vec<u8>, body_len: usize }
enum TransportCause { Timeout, Connect, Other }
```

- meeting **read** consumes `RawGatewayResponse` directly: exact
  `body_len` feeds budget accounting; non-2xx bodies are parsed as the
  typed error envelope of 6.5 (never flattened to generic text).
- meeting **abort** retries once **iff** the failure is
  `TransportCause::Timeout`; `Connect`/`Other` report immediately (no
  retry).
- No HTTP/auth/timeout logic is duplicated into `meeting.rs`; the
  single choke point stays in `gateway_client.rs` (§11 lists this full
  responsibility).

### 6.7 Self-describing output and untrusted framing

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
confirmation state and the epoch-qualified re-read command
("re-read this span: `--range A..B --epoch E`"); for empty
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
the agent runs the CLI (whose own rendering is governed by §6.7).
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
| `garyx-channels/src/feishu/types.rs` | 3 event structs | additive |
| `garyx-channels/src/feishu/ws.rs` | 3 dispatch branches (invited/activity/ended) → sink | additive |
| `garyx-channels/src/feishu/client.rs` | `bots_join`/`bots_leave` + adapter (no events pull, rev21) | additive |
| `garyx-channels/src/plugin.rs`, `feishu.rs` | 1 constructor dependency | additive |
| `garyx/src/commands/gateway.rs` | production `BuiltInPluginDiscoverer` construction (initial boot **and** `rebuild_channel_plugins` hot-reload) injects the same production `MeetingEventSink`; the no-op sink is for tests/non-gateway assemblies only | additive |
| `garyx-gateway/src/meetings/` (new) | service, coordinators, log writer/repair, locks, index, routes | new module |
| `garyx-gateway/src/garyx_db/mod.rs` | 3 tables (meetings, meeting_invite_keys, meeting_read_cursors) + CRUD incl. admission-key cascade | additive |
| `garyx-gateway/src/route_graph.rs` | 6 routes | additive |
| `garyx-gateway/src/composition/*` | service wiring + sink injection | additive |
| `garyx/src/commands/meeting.rs`, `cli.rs`, `main.rs` | CLI subcommand (incl. abort one-retry-on-timeout, typed-error envelope parsing from raw body) | additive |
| `garyx/src/commands/gateway_client.rs` | shared `RawGatewayResponse {status, raw_body, body_len}` primitive + typed `TransportCause {Timeout, Connect, Other}`; existing JSON helpers wrap it (budget accounting, typed envelopes, timeout-only retry all build on this) | additive |
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

**Push capture (rev21):** activity batch with mixed
transcript/chat/participant_left items normalizes per the pinned
shapes (fixtures copied from mino_server test data, sanitized);
redelivered event (same event_id / same sentence_ids) is a dedup no-op;
bare-integer ids parsed without f64 precision loss; activity for
unknown/terminal entities dropped with log; participant_left of the
bot's own open id triggers the end path; trailing activity during
finalizing is captured, after finalized is dropped; command executed
immediately when idle vs after the current batch commit.

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

**Budget (R7-02, RR8-01, RR15-01):** `--max-bytes 4096` with a 32 KiB
line → single segment served with the overshoot note; the RR15-01
counterexample pinned: two newline-heavy segments whose JSON is
48 KiB (fits a 64 KiB cap → both claimed) while their human rendering
is 96 KiB (irrelevant to the claim; CLI stdout exceeds the JSON budget
by design); floor rejection (0/1/4095); no zero-progress token in any
case; **pending-replay indivisibility**:
64 KiB/JSON claim → response lost → 4 KiB/human retry re-serves the
entire span with the replay note; concurrent winner/loser with different
budgets both deliver the winner's full span; new claims capped by
`read_page_bytes` regardless of requested budget.

**Snapshots/index:** `--full` across pages pinned to one snapshot under
concurrent appends; sliding renewal across >10 min total stream;
resume <10 min inactivity works; >10 min inactivity → typed `token_expired` with server-built restart
command (verified for `--range 100..200`: restart names 100, not the
token's next_seq); CLI prints it and exits nonzero (no auto-restart);
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
pages server-side. **Epoch algebra (RR10-01/03):** new reader created at
epoch>0 confirms successfully; preflight→rollover→claim returns
content-loss (epoch trichotomy); old-epoch receipt → content-loss vs
same-epoch stale receipt → idempotent no-op (distinct assertions);
epoch>0 ordinary fetch/confirm round-trip; header-printed
`--range … --epoch E` command re-executed verbatim after rollover
returns content-loss, not new-epoch content; token/range epoch
consistency. **S4(b) tail loss:** 3-page backlog, crash before P1
commit, recovery at GraceExpired → all three pages lost, no corruption,
loss logged. **ACK/dedup (RR10-04, RR11-04):** ACK send failure → processing
continues; invite/ended bypass the in-memory dedup cache; immediate
redelivery after a failed admission insert is admitted by the durable
key (not swallowed by the cache); redelivery after successful admission
is a unique-key no-op. **Epoch guards (RR11-01):** no-cursor-row
rollover between preflight and ensure → guarded INSERT inserts nothing →
content-loss; pending re-serve and empty-span branches verify the epoch
triple before serving; rollover with an existing new-epoch pending is
untouched by stale requests. **CLI parser (RR11-02):** header-printed
range/epoch and continuation commands parse and execute verbatim;
`--epoch` rejected with `--full`/incremental. **Hanging join
(RR11-03):** a never-returning join future is cancelled by deadline
abort, admin abort, and registry replacement; restart during a hung
attempt resumes retrying to the same absolute deadline.
**content_state pairing:** both illegal combinations rejected by
CHECK.

**Recognition preflight (RR8-03):** first cold terminal read hitting
`index_building` creates no cursor row; successful retry creates it;
lost-response-after-pending keeps the row (distinct from failed first
read).

**Stalled scoping (RR8-05):** terminal + unregistered account shows no
`no_client`; auth-failed finalizing → finalized clears the failure pair;
joining/finalizing/aborting statuses render correctly in read/list
output.

**WS path / loss rule (S1, RR12-01, RR13-03):** fixture records which
path carries invite/ended; raw-text carriage asserts the slice-2 halt
condition; the durable-fact rule is exercised via external fault
observation (no in-band "logged" guarantee asserted):
crash-before-insert with ACK success / ACK error / ACK in-flight ×
redelivery / no-redelivery matrices; insert-retries-exhausted;
redelivery after committed insert (no-op).

**Normative DDL execution (RR13-01, RR14-01):** the DDL block is
extracted verbatim from this document and executed against the target
SQLite **twice on the same database** (idempotent boot), plus a real
`GaryxDbService` reopen; the same test exercises both pairing CHECKs,
the reader byte-length CHECK, and FK cascade (meetings → cursors and
invite keys).

**Invite keys (RR14-02):** distinct event id folded into an active
entity → terminal → same event redelivered → no-op (key row survives
entity lifecycle until deletion); post-delete redelivery creates a
fresh entity; two distinct events admitted concurrently (one entity,
two key rows).

**Wire schema / continuation (RR14-03, RR15-02):** real CLI→HTTP body
assertions for all three modes plus token-only continue; field-
combination 400s (epoch with full; token with explicit range/epoch;
missing reader_id for incremental; stateless modes reject reader
identity); missing `GARYX_THREAD_ID` without `--thread` errors;
explicit `--thread` override honored; range first request → printed
resume command → real CLI parser → HTTP body → next page round-trip;
token-vs-path-entity and token-mode mismatches; pending claimed via
JSON re-served under human rendering (same span — format is not part
of the claim); NDJSON paging in `--json`.

**Abort domain (R18-01, R19-01):** deterministic interleaves —
coordinator paused after batch drain before CAS: retry joins the
in-flight operation, answered at the CAS outcome while leave hangs (no
second queue entry, no 10 s starvation); handler cancelled between
operation create and enqueue → operation invisible/rolled back, no
orphan waiters; coordinator channel closed at enqueue → waiters
completed with the typed retryable error; concurrent retries injected
after CAS but before waiter drain, after drain but before operation
removal, and after removal — each ends with the actual `200/409/404`
and never a second queued command; first request timing out with end
queued in the same batch → retry gets 409; retry after delete → 404.

**Transport seam (R18-02):** timeout triggers exactly one extra abort
attempt; connection-refused triggers none; nested 400
code/restart_command survives (not flattened); success `body_len`
equals actual body bytes.

**Abort response (RR15-03, RR16-02):** end-to-end through the real CLI
helper — full state/response table exercised (200 CAS / 200 no-op /
409 finalizing / 404 deleted / end-wins 409); slow page (9.9 s fetch +
delayed fdatasync + SQLite busy) → first request times out at the CLI,
retry returns idempotent 200; HTTP disconnect does not cancel the
admitted command; waiting duplicate aborts answered before the leave
attempt; entity converges to aborted within the leave bound.

**Budget accumulation (RR16-01, R17-01):** full-DTO round-trip (every
meta field asserted, segment DTO isomorphic to the seg schema);
two-page cumulative budget (50 KiB page then remaining 14 KiB — second
request carries the saturating remainder); **second page whose first
segment exceeds the remainder** → served with notes entry (per-page
exception, cumulative total overshoots the target); remainder < 4096
stops with resume command; raw-body byte counting asserted against the
extended helper; the 48 KiB-JSON / 96 KiB-human counterexample pinned
(both segments claimed; stdout exceeds JSON budget legally).

**Token expiry (RR16-03):** expired range token → typed 400 → CLI
prints the original command and exits nonzero; checksum/path/mode
validation precedes expiry check.

**Leave bound (RR14-04):** a never-returning `leave()` — entity reaches
`aborted` within the timeout bound, exactly one attempt observed.

**Hot reload (RR13-04):** real `rebuild_channel_plugins` cycle — old
channel unregisters, new channel registers/replaces, existing
non-terminal entities receive the register nudge, and a fresh invite
admissions successfully after reload (production sink present on both
construction paths; no silent no-op sink in gateway assemblies).

**Remote join / leave (RR12-02, RR13-02):** delayed-success-after-cancel
(second attempt converges on the same meeting id per pinned fixture);
crash-after-remote-success-before-CAS → boot re-join converges;
live-abort with client + id fires one best-effort leave (failure
logged, not retried); joining-abort attempts no leave (empty id);
no-client live abort terminates locally only; crash between abort CAS
and leave skips it (documented sign-off behaviors asserted); attempt
pacing has no hot loop on immediate failure.

**Delete idempotency reset (RR12-03):** same event id redelivered after
terminal → unique-key no-op; after delete → fresh entity (documented S2
behavior); distinct new event after delete → fresh entity.

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
