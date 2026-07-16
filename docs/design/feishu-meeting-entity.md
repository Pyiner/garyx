# Feishu meeting entity

Status: draft for adversarial review
Author: gary (design), based on #TASK-2322 research and two seam surveys
Supersedes: `docs/design/feishu-group-listen-mode.md` (group listening product
surface is cancelled; that document's passive-transcript machinery is NOT used
by this design — see Section 3.1)

## 1. Summary

When someone invites the Garyx bot into an ongoing Feishu meeting, the bot
joins, starts polling in-meeting events, and materializes a first-class
**meeting entity** in Garyx. While the meeting is live the entity grows
incrementally (transcript utterances, in-meeting chat, share events). When the
meeting ends the entity is **finalized** and becomes immutable.

The user can open a conversation about the entity at any time (live or
finalized). The agent never receives meeting content inline and never receives
a raw file path. It receives a **pointer**: entity id, status, metadata, and
one line of CLI usage. Content is read exclusively through a new CLI command,
`garyx meeting read`, which resolves the calling thread from `GARYX_THREAD_ID`,
keeps a **per-(entity, thread) read cursor on the server**, returns only the
increment on repeat reads, and self-describes its output ("this is an
increment, segments 12–18 of 18; use `--full` for everything").

Meeting content lives outside the thread transcript. The entity is its own
source of truth (same standing as the capsule HTML store). Thread ledgers only
ever carry lightweight reference rows. None of the passive-row concurrency
machinery from the superseded group-listen design is needed.

## 2. Product contract (user-approved, 2026-07-16)

1. Meetings only. No group-chat listening product surface.
2. Start: receiving an in-meeting invite for the bot
   (`vc.bot.meeting_invited_v1`) joins the meeting and starts polling. A
   manual CLI trigger exists as a debug path, not a product surface.
3. Live: a meeting entity accumulates content in real time, stored as files an
   agent could read — but agents do not read the files directly (see 4).
4. Read protocol: agents MUST read entity content through the CLI. The entity
   records which thread has read what. A second read from the same thread
   returns the increment by default. Output explicitly states that it is an
   increment and how to fetch everything.
5. End: receiving the meeting-ended signal stops polling and freezes the
   entity permanently.
6. Reference continuity: when the same thread references the entity again, the
   system recognizes the prior reference and defaults to incremental reads.

## 3. Verified repository baseline

Every claim was verified against the working tree at design time.

| # | Fact | Evidence |
|---|---|---|
| B1 | Feishu WS dispatch handles only `im.message.receive_v1`; all other event types are dropped in the `else` branch | `garyx-channels/src/feishu/ws.rs:1049-1072` |
| B2 | `FeishuChannel::new(config, router, bridge, dispatcher, public_url)`; deps are injected per-account into `FeishuRuntimeContext` | `garyx-channels/src/feishu.rs:370`, `ws.rs:38-69` |
| B3 | Channel construction goes through `BuiltInPluginDiscoverer::with_dispatcher(...)`; gateway-implemented behavior reaches channels only via injected traits (`ThreadCreator` precedent) | `garyx-channels/src/plugin.rs:3235,3322`, `garyx-router/src/router/contracts.rs:12-27` |
| B4 | `FeishuClient` owns tenant-token refresh (double-checked, single-flight, 5 min margin) and is `Clone` (Arc-shared token); new REST calls follow `send_message_to_target` | `garyx-channels/src/feishu/client.rs:127,226,243,291` |
| B5 | Long-lived background service precedent: `CronService` — `tokio::select!` loop + stop channel + `Weak<AppState>` backref + stale-`Running`-state reset on boot | `garyx-gateway/src/cron.rs:633,713-796,799-851` |
| B6 | Entity storage precedent: capsules — content on disk (`~/.garyx/capsules/{uuid}.html`, atomic write-then-rename, 5 MiB cap), metadata in SQLite STRICT table with PRAGMA-based column migration | `garyx-gateway/src/capsules.rs:158,273`, `garyx-gateway/src/garyx_db/mod.rs:325,790-932,2645,3559` |
| B7 | Tool-result → control record → render card chain: `capsule_attached` control records are extracted by the bridge and rendered by the server reducer | `garyx-bridge/src/multi_provider/persistence.rs:364,394`, `garyx-models/src/transcript_render_state.rs:1192-1280` |
| B8 | `GARYX_THREAD_ID` is injected into every agent process env by the bridge; CLI precedent reads it via `env_nonempty` | `garyx-bridge/src/gary_prompt.rs:154`, `garyx/src/commands/gateway_client.rs:439`, `commands/task.rs:233` |
| B9 | CLI→gateway pattern: `gateway_endpoint` (base URL + bearer token from `garyx.json`) + shared retrying JSON helpers | `garyx/src/commands/gateway_client.rs:73,101,220,294-387` |
| B10 | Prompt context blocks are prepended by the bridge (`<garyx_thread_metadata>`), not by channels; attachment instructions use the same seam | `garyx-bridge/src/gary_prompt.rs:36,73,125` |
| B11 | Event bus: `EventStreamHub` broadcast + per-thread SSE; gateway-side services can publish directly | `garyx-gateway/src/composition/event_stream_hub.rs:6-49`, `routes.rs:1938-2054` |

External platform facts (lark-cli 1.0.70 + internal onboarding manual,
2026-07 field research; scopes `vc:meeting.bot.join:write` and
`vc:meeting.meetingevent:read` verified live on the production app):

- `POST /open-apis/vc/v1/bots/join` takes the 9-digit `meeting_no` (+ optional
  `call_id` forwarded from the invite event) and returns the long numeric
  `meeting.id` used by all later calls. Tenant token only.
- `GET /open-apis/vc/v1/bots/events` is **pull-based** (recommended cadence
  10–30 s), pages 20–100 with `page_token` continuation. Event types:
  `participant_joined/left`, `chat_received`, `transcript_received`,
  `magic_share_started/ended` (with `share_doc.title/url`).
- Transcript latency is ~30 s (batched at 5 s/100 items). Multi-party meetings
  only. The meeting owner must enable "allow agents to join" per meeting. The
  bot must actually be in the meeting to read events (`10005` otherwise).
  After the meeting ends there is a 5-minute grace window (`20001` after).
- Post-meeting minutes/notes are not auto-authorized to the bot (platform fix
  in progress) — final summaries in Phase 1 derive from our own accumulated
  transcript, and minute ingestion is a future seam, not a dependency.
- `vc.bot.meeting_invited_v1` / `vc.bot.meeting_ended_v1` are push events that
  must be checked in the developer console event subscriptions (deployment
  prerequisite; a misspelled subscription silently receives nothing).

### 3.1 Relationship to the superseded design

`feishu-group-listen-mode.md` solved "external content flows into the thread
transcript". This design deliberately avoids that problem: meeting content
never enters any thread ledger, so the passive-commit gate, per-thread
serialization domain, deferred FIFO, rolling summary checkpoints, and delivery
suppression are all unnecessary. What survives from that effort is research
(#TASK-2322) about which contracts we must not violate — chiefly that we do
not write foreign rows into transcripts, which this design satisfies by
construction.

## 4. Entity model and storage

Follows the capsule split: metadata in SQLite, content on disk. Gateway owns
both.

### 4.1 SQLite

New STRICT table `meetings` (DDL batch alongside `capsules`,
`garyx_db/mod.rs:2645` pattern):

```
id                TEXT PRIMARY KEY        -- uuid_v7, Garyx entity id
feishu_meeting_id TEXT NOT NULL           -- long numeric id from bots/join
meeting_no        TEXT NOT NULL           -- 9-digit meeting number
topic             TEXT NOT NULL DEFAULT ''
account_id        TEXT NOT NULL           -- feishu account that joined
status            TEXT NOT NULL           -- 'live' | 'finalized' | 'aborted'
invited_by        TEXT NOT NULL DEFAULT '' -- open_id of inviter (metadata only)
segment_count     INTEGER NOT NULL DEFAULT 0
byte_size         INTEGER NOT NULL DEFAULT 0
poll_cursor       TEXT NOT NULL DEFAULT '' -- feishu page_token continuation
started_at        TEXT NOT NULL           -- RFC3339 UTC
ended_at          TEXT                    -- set on end signal
finalized_at      TEXT                    -- set when grace pull completes
created_at        TEXT NOT NULL
updated_at        TEXT NOT NULL
```

Indexes: `idx_meetings_updated(updated_at DESC)`, `idx_meetings_status(status)`.

New STRICT table `meeting_read_cursors` — **the server-side read state the
whole read protocol hangs on**:

```
meeting_id   TEXT NOT NULL
thread_id    TEXT NOT NULL
last_segment INTEGER NOT NULL DEFAULT 0   -- highest segment seq delivered
updated_at   TEXT NOT NULL
PRIMARY KEY (meeting_id, thread_id)
```

`status='aborted'` covers meetings where the bot was removed or polling
failed permanently before a normal end; content accumulated so far is kept
and the entity finalizes with a marker segment (see 5.4).

### 4.2 Content files

`~/.garyx/meetings/{entity_id}/` (new `default_meetings_dir()` in
`garyx-models/src/local_paths.rs`, sibling of `default_capsules_dir`):

- `transcript.md` — append-only, human/agent-readable, the canonical content.
  Structured as numbered **segments**; a segment is the unit the read cursor
  counts. One segment = one speaker-coalesced utterance block, one chat
  message, or one share/join/leave marker:

  ```
  ## [12] 10:32:05 张三 (transcript)
  我觉得这个方案的问题在于……

  ## [13] 10:32:41 李四 (chat)
  +1，另外看下这个文档

  ## [14] 10:33:02 (share) 张三 started sharing: 《Q3 规划》 https://…
  ```

  Speaker coalescing: consecutive `transcript_received` items from the same
  speaker within 60 s merge into one segment (segment is closed when the
  speaker changes, the gap exceeds 60 s, or a non-transcript event lands).
  Segment numbers are dense, monotonically increasing, and never rewritten.
- `events.jsonl` — raw normalized poll events, append-only, for audit and
  re-derivation. Not exposed to agents.
- Writes are append + fsync batches; `segment_count`/`byte_size`/`updated_at`
  in SQLite are updated after the file append succeeds (write-then-derive,
  same ordering discipline as the rest of the repo). On crash the file may be
  ahead of SQLite by a partial batch; boot recovery (5.5) re-counts segments
  from the file, so the file is truth for content and SQLite for lifecycle.

Raw absolute paths never appear in prompts, CLI output, or API responses;
agents address the entity only by id.

## 5. Ingestion pipeline

### 5.1 Event intake (channels crate)

- `types.rs`: add `VcBotMeetingInvitedEvent` / `VcBotMeetingEndedEvent`
  (serde-default structs, same envelope pattern as `ImMessageReceiveEvent`;
  invite carries `meeting_no`, `meeting_id`, `call_id`, inviter open_id —
  exact field names to be pinned from a captured sample during slice 1).
- `ws.rs` dispatch (`ws.rs:1049-1072`): two new `else if` branches ahead of
  the ignore branch. Existing `event_id` dedup cache applies unchanged.
- New injected trait, defined in `garyx-channels` (precedent B3), implemented
  by gateway:

  ```rust
  #[async_trait]
  pub trait MeetingEventSink: Send + Sync {
      async fn on_meeting_invited(&self, ctx: MeetingInviteContext, client: FeishuClient);
      async fn on_meeting_ended(&self, account_id: &str, feishu_meeting_id: &str);
  }
  ```

  `MeetingInviteContext` carries account_id, meeting_no, meeting_id, call_id,
  inviter, and the account's display config. The `FeishuClient` clone hands
  the gateway a ready, token-managed HTTP client (B4) so the gateway never
  re-implements Feishu auth. Threaded through `with_dispatcher` →
  `FeishuChannel::new` → `FeishuRuntimeContext` exactly like `dispatcher`
  (B2/B3). A no-op default implementation keeps non-gateway assemblies and
  tests compiling.

### 5.2 MeetingService (gateway crate)

New `garyx-gateway/src/meetings/service.rs`, shaped like `CronService` (B5):
`Arc<MeetingService>` on `AppState`, owning:

- the `MeetingEventSink` implementation;
- one polling task per live meeting (`tokio::select!` on a 30 s interval +
  stop channel), tracked in `HashMap<entity_id, JoinHandle>`;
- persistence via `state.ops.garyx_db` and the content-file writer;
- event publication to `EventStreamHub` (B11) with a new
  `meeting_update{entity_id, status, segment_count, updated_at}` payload for
  live UI refresh (list surfaces re-fetch on it; no per-segment push in
  Phase 1).

On `on_meeting_invited`:

1. Idempotency: if a `live` entity already exists for
   `(account_id, feishu_meeting_id)`, ignore the duplicate invite.
2. `POST bots/join` (meeting_no + call_id). On failure retry every 20 s for
   up to 5 minutes (matches the invite validity window observed in the field
   research), then give up recording nothing but a log line — no entity is
   created for a join that never succeeded.
3. Create the entity row (`status='live'`), create the content dir, write a
   header segment, spawn the poll loop.

Poll loop each tick: `GET bots/events` with stored `poll_cursor`, `page-all`
semantics; normalize → append `events.jsonl` → coalesce into segments →
append `transcript.md` → update SQLite counters + `poll_cursor` → publish
`meeting_update`. Poll errors are tolerated with exponential backoff within
the loop (30 s → 60 s → 120 s cap); `10005` (bot no longer in meeting) or
`20001` (meeting ended) short-circuit to the end path.

### 5.3 End detection and grace pull

Two triggers, whichever arrives first:

- push: `vc.bot.meeting_ended_v1` via the sink;
- pull: poll loop observes `20001`/ended (also covers missed push events —
  the poll loop is the reliability backstop, the push event only makes it
  faster).

End path: mark `ended_at`, do one final `bots/events` sweep (the 5-minute
grace window permits reading events for a meeting the bot attended), write a
closing marker segment (`## [N] meeting ended`), set `status='finalized'`,
`finalized_at`, stop and deregister the poll task. Finalized entities are
immutable: the writer refuses appends and the service never re-spawns a loop
for them.

### 5.4 Abort path

If the bot is removed mid-meeting (`10005` while the meeting is still live)
or polling fails permanently (backoff exhausted for > 10 minutes), the entity
finalizes with `status='aborted'` and a marker segment stating why and when.
Content accumulated so far remains readable through the normal protocol.

### 5.5 Restart recovery

Boot: `MeetingService::load()` scans `meetings WHERE status='live'`
(stale-Running reset precedent, `cron.rs:729-744`):

- re-count segments from `transcript.md` (file is content truth; heals the
  crash gap in 4.2);
- attempt one `bots/events` probe: still live → respawn the poll loop with
  the persisted `poll_cursor`; ended/grace-expired/`10005` → run the end or
  abort path.

### 5.6 Lifecycle state machine

```
                    invite event
                         v
   (no entity) ----> Joining ----join ok----> Live <-----------------+
                       | retry<=5min           | poll tick: append    |
                       | exhausted             | segments             |
                       v                       |                      |
                    (nothing)     ended push/pull   bot removed/      |
                                       |            poll dead         |
                                       v                 |            |
                                  GracePull               |     boot: probe
                                       |                  |     says still live
                                       v                  v            |
                                  Finalized           Aborted     (respawn)
                                  (immutable)        (immutable)
```

## 6. Read protocol (CLI-mediated, cursor on the entity)

This is the load-bearing product idea: **increment logic lives on the read
side, not the injection side.** Agents are stateless; the entity remembers
who read what.

### 6.1 CLI surface

New `garyx/src/commands/meeting.rs` (CLI pattern B9), clap subcommand
`garyx meeting`:

- `garyx meeting list [--json]` — entities, newest first: id, topic, status
  (`live`/`finalized`/`aborted`), segment span, updated_at.
- `garyx meeting read <entity_id> [--full] [--range A..B] [--thread <id>] [--json]`
  — the only content access path. Thread identity resolves from
  `GARYX_THREAD_ID` (B8); `--thread` overrides for debugging; if neither is
  present the command errors with guidance (cursorless anonymous reads are
  not offered — predictability over convenience; use `--range` for stateless
  inspection).
- `garyx meeting join <meeting_no> [--account <id>]` — manual debug trigger
  for the ingestion pipeline (contract item 2).

### 6.2 Gateway API

```
GET  /api/meetings                       -> list (id, topic, status, counts)
GET  /api/meetings/{id}                  -> metadata only
POST /api/meetings/{id}/read             -> { thread_id, mode: incremental|full|range, range? }
POST /api/meetings/{id}/join-debug       -> manual trigger
```

`/read` executes server-side, atomically per (meeting, thread) row:

1. Load cursor `c` for `(id, thread_id)` (absent ⇒ 0).
2. Resolve the slice: `incremental` ⇒ segments `(c, latest]`; `full` ⇒
   `[1, latest]`; `range` ⇒ `[A, B]` verbatim.
3. Cursor semantics: `incremental` and `full` advance the cursor to `latest`
   as of this read; `range` never moves the cursor (it is a peek).
4. Respond with the segment slice plus a self-description header (mode,
   span returned, total segments, live/finalized, cursor before/after).

Segments are extracted from `transcript.md` by segment number; the file's
dense numbering makes slicing a linear scan with no index (meetings are
small; an index is premature).

### 6.3 Self-describing output (contract item 4)

Human format (default; `--json` returns the same data structured):

```
── meeting entity 019f… ─ 《Q3 规划会》 ─ LIVE, updated 10:35:12 ──
Incremental read for this thread: segments 12–18 of 18
(first read returned 1–11; meeting is live, re-read later for more)
To read everything:      garyx meeting read 019f… --full
To re-read a span:       garyx meeting read 019f… --range 5..9  (does not move your cursor)
─────────────────────────────────────────────

## [12] 10:32:05 张三 (transcript)
…
```

First read from a thread states "first read: full content, segments 1–N".
Finalized entities say `FINALIZED at <time>`; aborted ones state the abort
reason line. Empty increments return the header plus "no new segments since
your last read at [11]" — never an empty string, so the agent always learns
the current state.

### 6.4 Concurrency and edge rules

- Cursor row upsert is a single SQLite transaction; two concurrent reads from
  the same thread both succeed, and the cursor lands at `latest` (segment
  delivery may overlap — acceptable; segments are idempotent content, and
  agent turns within one thread are effectively serial anyway).
- Reading a `live` entity races appends benignly: the slice is whatever was
  durable at query time, and the header says the meeting is live.
- Deleting a thread leaves cursor rows behind; they are garbage-collected by
  the same purge path that nulls `capsules.thread_id`
  (`garyx_db/mod.rs:2797` precedent — delete rows instead of nulling).

## 7. Pointer injection and references

### 7.1 Pointer block

When a conversation is started from a meeting entity (7.2) the message
metadata carries `meeting_ref: {entity_id}`. The bridge prepends a
`<garyx_meeting_context>` block next to `<garyx_thread_metadata>` (seam B10):

```
<garyx_meeting_context>
entity_id: 019f…
topic: Q3 规划会
status: live            (updated: 2026-07-16 10:35:12)
segments: 18
this_thread_cursor: 11  (7 unread segments)
read: garyx meeting read 019f…          # returns unread increment
</garyx_meeting_context>
```

No file paths, no content. The block is rebuilt fresh each turn while the
ref is active, so cursor/segment numbers are current. The cursor lookup is a
point read; if it fails the block renders with `cursor: unknown` rather than
blocking the turn.

### 7.2 Where refs come from

- **Entity card → "chat about this meeting"** (primary): creates a new thread
  whose first turn carries `meeting_ref`, or continues the thread already
  associated with the entity if the user picks it from the card. Threads
  remember active refs in thread-record metadata
  (`meeting_refs: [entity_id]`); every subsequent user turn in that thread
  re-injects the context block (this is the "recognize the prior reference"
  contract item — recognition is server state, not agent memory).
- **CLI discovery** (secondary): any agent can `garyx meeting list` and read
  entities without a ref; cursors work identically. The ref only adds the
  per-turn context block.

### 7.3 In-thread cards

Reuse the capsule three-stage chain (B7): when the MeetingService creates or
finalizes an entity, and a thread holds a ref to it, the gateway appends a
`meeting_attached` control record through the same seam as
`capsule_attached`; the server reducer renders a meeting card row
(`RenderMeetingCard { entity_id, topic, status, action: attached|finalized }`).
Desktop/iOS dumb-render it like capsule cards. (Slice 3; the protocol works
without cards.)

## 8. UI surfaces

Mirrors capsules; details follow `garyx-product-ui` during implementation.

- **Desktop**: "Meetings" gallery in the left rail (list: live badge, topic,
  duration, segment count; live entries re-fetch on `meeting_update` SSE).
  Card actions: "Chat about this meeting" (7.2), delete (finalized only).
  Chat renders `meeting_attached` cards.
- **iOS**: same catalog surface via gateway-scoped stale-while-refresh cache;
  route state and mapping in `GaryxMobileCore` with SwiftPM tests; no local
  render derivation (render_state rule).
- Entity deletion removes DB rows (meetings + cursors) and the content dir;
  live entities cannot be deleted (stop happens via meeting end/abort).

## 9. Configuration and deployment prerequisites

- `FeishuAccount`: `#[serde(default = "default_true")] pub meeting_entities:
  bool` — per-account kill switch, default on (feature is invite-driven and
  inert unless someone invites the bot). Serde-default keeps old configs
  loading (config-compat rule).
- Gateway: `#[serde(default)] pub meetings: MeetingConfig` on `GatewayConfig`
  (`GatewayAutoUpdateConfig` pattern) holding `poll_interval_secs` (default
  30, clamp 10–120) and `join_retry_window_secs` (default 300).
- **Developer console (manual, blocking deploy):** subscribe
  `vc.bot.meeting_invited_v1` and `vc.bot.meeting_ended_v1` events for the
  production app and publish a new app version. Scopes are already granted
  and verified live. Event names must be exact.
- No new secrets. Public-repo hygiene: all fixtures use synthetic ids.

## 10. Failure and observability

- Every lifecycle transition logs at info with entity id + feishu meeting id.
- Poll failures: warn with backoff state; abort path logs the terminal cause.
- `garyx meeting list` shows status directly; an `aborted` entity is the
  user-visible signal that capture degraded.
- Metrics via logs in Phase 1 (poll latency, segments/tick, join failures);
  no new metrics infrastructure.

## 11. Implementation impact map

| Area | Change |
|---|---|
| `garyx-models/src/local_paths.rs` | `default_meetings_dir()` |
| `garyx-models/src/config.rs` | `FeishuAccount.meeting_entities`, `GatewayConfig.meetings` |
| `garyx-channels/src/feishu/types.rs` | 2 event structs |
| `garyx-channels/src/feishu/ws.rs` | 2 dispatch branches; `MeetingEventSink` in runtime context |
| `garyx-channels/src/feishu/client.rs` | `bots_join`, `bots_events` REST methods |
| `garyx-channels/src/plugin.rs`, `feishu.rs` | sink threading through constructor |
| `garyx-gateway/src/meetings/` | new module: service, poll loop, segment writer, recovery |
| `garyx-gateway/src/garyx_db/mod.rs` | `meetings`, `meeting_read_cursors` tables + CRUD + purge hook |
| `garyx-gateway/src/route_graph.rs`, `meetings/routes.rs` | 4 routes |
| `garyx-gateway/src/composition/*` | service wiring, sink injection |
| `garyx-bridge/src/gary_prompt.rs` | `<garyx_meeting_context>` builder |
| `garyx-bridge` persistence | `meeting_attached` control record extraction (slice 3) |
| `garyx-models/src/transcript_render_state.rs` | meeting card reduction (slice 3) |
| `garyx/src/commands/meeting.rs`, `cli.rs`, `main.rs` | CLI |
| Desktop / iOS | gallery + cards (slice 3) |

## 12. Test plan (headless-first)

- **Fixture-driven ingestion**: captured (sanitized) `bots/events` JSON pages
  drive the normalizer — assert segment coalescing (speaker merge, 60 s gap,
  chat/share interleave), dense numbering, `events.jsonl` fidelity. Captured
  invite/ended envelopes drive WS dispatch → sink calls.
- **Lifecycle**: end-by-push vs end-by-poll; grace sweep appends; abort on
  `10005`; immutability after finalize (writer refuses).
- **Recovery**: crash between file append and SQLite update → boot re-count
  heals; live probe respawn vs finalize-on-probe.
- **Read protocol** (pure SQLite + file, no Feishu): first read full;
  repeat read increments; `--full`/`--range` cursor semantics; empty
  increment message; concurrent reads land cursor at latest; per-thread
  isolation of cursors.
- **CLI contract**: `GARYX_THREAD_ID` resolution, override, missing-id error;
  self-description header golden tests (human + `--json`).
- **Prompt block**: `<garyx_meeting_context>` builder unit tests (ref present,
  cursor unknown, finalized).
- Rust: `cargo test -p garyx-gateway --lib`, `-p garyx-channels`, tier1 fast
  loop. No UI tests required before slice 3; slice 3 mobile logic lands in
  `GaryxMobileCore` with SwiftPM tests.

## 13. Delivery slices

1. **Entity core + read protocol** (no Feishu dependency): tables, content
   writer, read API + cursors, `garyx meeting list/read`, fixture tests.
   Gate: read-protocol test suite green; a hand-seeded entity is readable
   incrementally from two threads with independent cursors.
2. **Ingestion**: event structs, WS branches, sink, join + poll loop, end/
   abort/recovery, `meeting join` debug command, console subscriptions
   checked. Gate: a real invited meeting produces a live→finalized entity
   with correct segments; restart mid-meeting resumes.
3. **Experience**: pointer block + thread refs, `meeting_attached` cards,
   desktop/iOS galleries. Gate: card → chat → agent reads increment → answer
   cites new segments, end to end.

## 14. Open questions (non-blocking, defaults chosen)

- Retention: entities live until manually deleted (no auto-expiry) — default
  accepted unless storage becomes a concern.
- Multiple accounts invited to the same meeting: one entity per
  (account, meeting) pair; cross-account dedup is out of scope.
- `meeting_activity_v1` (participant activity push) is ignored in Phase 1;
  the poll loop already captures joins/leaves.
