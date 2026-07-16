# Feishu meeting entity

Status: revision 5 — scope reset per user directive (2026-07-16): the meeting
entity is an **orthogonal side-system**; existing runtime flows are not
modified. Capture/read-protocol correctness results from review rounds 1–4
are retained; every integration touchpoint introduced during those rounds is
withdrawn.
Author: gary (design)
Supersedes: `docs/design/feishu-group-listen-mode.md` (group listening
cancelled). Meeting content never enters thread ledgers.

## 1. Summary

When someone invites the Garyx bot into an ongoing Feishu meeting, the bot
joins, polls in-meeting events, and materializes a **meeting entity** in a
self-contained subsystem: its own SQLite tables, its own on-disk segment
log, its own gateway routes, its own CLI. Capture is durably checkpointed at
bounded tick granularity; when the meeting ends, capture drains the
platform's grace window and the entity becomes immutable.

Agents read entities exclusively through `garyx meeting read` — a tool-call
like any other CLI use, requiring **zero changes** to the bridge, providers,
routing, transcripts, or rendering. The server keeps a per-(entity, thread)
cursor: a thread's first read returns content from the start and implicitly
*is* the "this thread has seen this entity" memory; every later read
returns the increment. Delivery is fetch/confirm two-phase inside one CLI
invocation — at-least-once, never silently skipped.

Referencing an entity from a conversation is a **text convention**, not a
protocol: UI surfaces prefill a line of text containing the entity id and
the read command. The message travels every existing path unchanged; the
agent reads the id and runs the CLI.

## 2. Product contract (user-approved, 2026-07-16; scope reset same day)

1. Meetings only. No group-chat listening.
2. Start: an in-meeting invite admits, joins, polls. `garyx meeting join`
   is a debug path.
3. Live: near-real-time accumulation (~30 s platform transcript latency).
4. Read protocol: CLI-only content access; per-thread server cursors;
   incremental by default; self-describing output.
5. End: meeting-ended signal stops capture; entity freezes.
6. Reference continuity: the cursor row itself is the recognition of a
   prior reference — no other machinery.
7. **Orthogonality (scope reset):** no existing flow changes. No prompt
   injection, no bridge/provider/dispatch instrumentation, no
   render-state/reducer changes, no thread-lifecycle hooks. The only
   additive touchpoints outside the new subsystem are: two WS event
   branches in the feishu channel + an injected sink trait (new events,
   ignored today), new CLI subcommands, new gateway routes/tables, and
   (slice 3) a gallery UI whose "chat" action merely prefills text.

Product sign-off items (accepted limits):

- Feishu WS ACKs before processing; an invite is lost iff the process dies
  or the admission insert exhausts 3 bounded retries in the ACK→admission
  window (error-logged).
- One entity per admission: re-invites after terminal/deletion create new
  entities.
- Final ~30 s of speech becomes readable during the grace drain.
- A crash loses at most the current bounded in-memory tick; the loss is
  healed by idempotent re-pull.
- An agent learns about an entity from the referenced text in the
  conversation, from `garyx meeting list`, or from its own memory — there
  is no automatic per-turn context injection. If the user never mentions
  the entity and the agent never lists, the agent will not know it exists;
  that is accepted and intended.

## 3. Verified repository baseline

The subsystem touches so little that only these remain load-bearing:

| # | Fact | Evidence |
|---|---|---|
| B1 | WS ACKs before processing; only `im.message.receive_v1` handled; 30 min event-id dedup | `garyx-channels/src/feishu/ws.rs:968,1040,1049-1072`, `feishu.rs:95` |
| B2 | `FeishuChannel::new(config, router, bridge, dispatcher, public_url)` → `FeishuRuntimeContext`; injection precedent for one more constructor dependency | `feishu.rs:370`, `ws.rs:38-69`, `plugin.rs:3235,3322` |
| B3 | gateway depends on channels; a channels-defined, gateway-implemented trait compiles (first such trait) | `garyx-gateway/Cargo.toml:20` |
| B4 | `FeishuClient` is `Clone` but `pub(crate)` — cross-crate use needs a public trait object | `client.rs:127` |
| B5 | `CronService` background-service shape (select loop, stop channel, `Weak<AppState>`, boot state reset) | `cron.rs:633,713-796,799-851` |
| B6 | Capsule precedent: disk content + SQLite STRICT metadata + migration; gateway-owned entity storage with CLI-less HTTP surface | `capsules.rs:158,273`, `garyx_db/mod.rs:2645,3559` |
| B7 | `GARYX_THREAD_ID` is injected into every agent process env; CLI precedent reads it | `gary_prompt.rs:148-154`, `commands/task.rs:233` |
| B8 | CLI→gateway pattern (endpoint, bearer, retrying helpers; 10 s mutation timeout → paging budget) | `gateway_client.rs:15,73,101,220` |

Withdrawn from the baseline (no longer needed): everything about
`start_agent_run`/`add_streaming_input` internals, queued-input allowlists,
`gary_prompt` context blocks, render-state reducers, thread delete/archive
transaction paths, and event-stream/SSE routing. The subsystem does not
integrate with any of it.

External platform facts: `POST /open-apis/vc/v1/bots/join` (9-digit
`meeting_no` + optional `call_id` → long `meeting.id`; tenant token);
`GET /open-apis/vc/v1/bots/events` (pull, 10–30 s cadence, opaque
`page_token` continuation — stored and resumed from, never compared);
event types `participant_joined/left`, `chat_received`,
`transcript_received`, `magic_share_started/ended`; ~30 s transcript
latency in 5 s/100-item batches; multi-party only; per-meeting owner
switch; `10005` = bot not in meeting; 5-minute post-end grace window,
`20001` = window over. Typed errors:
`NotInMeeting | GraceExpired | RetriableTransport | Other(code, msg)`.
Console must subscribe `vc.bot.meeting_invited_v1` /
`vc.bot.meeting_ended_v1` (deployment prerequisite).

### 3.1 Sample-pinning gate

Slice 2 opens by capturing sanitized fixtures (invite/ended envelopes, join
response, events pages incl. grace-window reads and the `20001` response).
Pinned facts: invite payload fields; joining-stage identity; existence or
absence of an in-band ended item; grace-window read behavior. Mismatch
stops slice 2 for a design amendment. Slice 1 has no Feishu dependency.

## 4. Entity model and storage

### 4.1 SQLite

`meetings` (STRICT; timestamps UTC RFC3339 `Z`):

```
id / account_id / meeting_no / feishu_meeting_id ('' until join)
invite_event_id / call_id / topic / invited_by
status CHECK(status IN ('joining','live','finalizing','aborting','finalized','aborted'))
status_detail / stalled_reason ('' | 'no_client' | 'auth_failed' | 'transport')
end_source ('' | 'push' | 'poll_ended'(if pinned) | 'grace_expired')
join_deadline_at / grace_deadline_at
poll_cursor (cache; the log checkpoint chain is truth)
closed_segment_count / byte_size (caches)
started_at / ended_at / finalized_at / created_at / updated_at
```

Uniqueness: `UNIQUE(invite_event_id)`; partial uniques
`(account_id, meeting_no)` and `(account_id, feishu_meeting_id) WHERE
feishu_meeting_id <> ''` over
`status IN ('joining','live','finalizing','aborting')` — one active entity
per meeting per account; terminal/deleted never blocks a new admission.

`meeting_read_cursors` (STRICT):

```
meeting_id REFERENCES meetings(id) ON DELETE CASCADE
thread_id      -- opaque caller identity; NOT validated against thread_records
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

**Deliberate non-integration:** `thread_id` here is an opaque reader
identity supplied by the CLI (from `GARYX_THREAD_ID`). The subsystem does
not validate it against `thread_records`, does not hook thread deletion,
and does not care if the thread later disappears — a dead thread's cursor
row is inert data cleaned up when the entity is deleted (FK cascade).
This is the price and the point of orthogonality.

Indexes: `idx_meetings_updated(updated_at DESC, id)` (keyset pagination),
`idx_meetings_status(status)`.

There is no refs table. Reference recognition **is** the cursor row.

### 4.2 Content: bounded-tick checkpointed segment log

Unchanged from revision 4 (review-hardened): one
`~/.garyx/meetings/{entity_id}/segments.jsonl` with `seg`/`ckpt` lines;
write-time splitting of >32 KiB items into independent seqs (`"cont":true`
render hint); bounded ticks (close at `has_more=false` or 10 pages / 1000
items / 1 MiB / 10 s, whichever first) with the single durability sequence
close-coalescing → `seg*` → `ckpt` → `fdatasync` → SQLite cache
transaction; every tick (empty ones included) checkpoints; commands are
serviced at tick closes; dedup by `sources` platform ids; boot repair
(non-terminal/dirty logs only) truncates torn tails, resumes from the last
valid `ckpt`, rebuilds counters, and builds the in-memory offset index in
the same pass. Markdown is a render format; no on-disk Markdown, headers,
or end markers.

### 4.3 Entity deletion

Unchanged from revision 4: terminal-only; under the entity I/O lock
rename→tombstone, DB delete (cascades cursors), remove tombstone; missing
content dir = legal empty entity (skip rename, DB protocol only); boot
reconciles tombstones **before** log repair (tombstone+row → restore;
tombstone w/o row → remove; bare orphan → remove, logged).
`DELETE /api/meetings/{id}` (409 non-terminal) + `garyx meeting delete`.

## 5. Ingestion

Unchanged from revision 4 — the whole pipeline lives inside the subsystem:

- **5.1 Client seam/registry:** `MeetingPlatformClient` +
  `MeetingEventSink` (`register_client`/`unregister_client`/
  `on_meeting_invited`/`on_meeting_ended`) defined in a new
  `garyx-channels/src/meeting_sink.rs`; adapter over `FeishuClient`
  implemented inside garyx-channels; gateway implements the sink;
  threaded through the existing constructor path (B2). Admission insert =
  3 bounded retries, error-logged; nothing else on the WS loop. No-op sink
  default keeps other assemblies compiling. The **only** change to
  existing channel code is two new `else if` event-type branches and one
  more constructor dependency.
- **Stalled semantics per state:** joining's absolute deadline always
  applies (aborts even while stalled); live never auto-aborts for stalled
  reasons (only `NotInMeeting`); finalizing refuses abort and finalizes at
  `grace_deadline_at` even if stalled; admin
  `garyx meeting abort` / `POST /api/meetings/{id}/abort` applies to
  joining|live only; `stalled_reason` persisted and listed.
- **5.2 Coordinator:** per-entity task over durable CAS with intent stages
  (`aborting`/`finalizing`); admission via unique keys; lazy `ensure_dir`;
  join retries every 20 s to the absolute deadline; terminal-affecting
  commands resolved at tick closes with priority **end > abort**; join
  succeeding during grace is legal (polls return data until
  `GraceExpired`).
- **5.3 End/grace:** push (or pinned poll_ended) → `live→finalizing`,
  `grace_deadline_at = now + 4 min`, drain to the deadline (no quiescence
  shortcut), then `finalized`. `GraceExpired` while live → finalize
  immediately; while finalizing → accelerate completion. `NotInMeeting`
  while finalizing → complete early. Unknown-entity end signals: logged,
  dropped.
- **5.4 State machine and 5.5 boot recovery:** as revision 4 (JOINING/LIVE
  resume loops; FINALIZING resumes drain; ABORTING finishes flush;
  terminal rows never spawn coordinators).

## 6. Read protocol

Unchanged from revision 4 in substance; restated without any integration
references:

- **CLI:** `garyx meeting list` (keyset-paged);
  `garyx meeting read <id> [--full | --range A..B] [--thread <id>] [--json]`;
  `join`/`abort`/`delete`. Incremental (default) requires thread identity
  (`GARYX_THREAD_ID` env — already present in every agent shell — or
  `--thread`); `--full`/`--range` are stateless paged snapshot peeks (no
  identity, no cursor).
- **API:** `GET /api/meetings` (keyset `(updated_at, id)` + snapshot
  boundary token), `GET /api/meetings/{id}`,
  `POST /api/meetings/{id}/read`, `POST /api/meetings/{id}/read/confirm`,
  `POST /api/meetings/{id}/abort`, `DELETE /api/meetings/{id}`,
  `POST /api/meetings/{id}/join-debug`.
- **Locking/index:** per-entity `RwLock` over all states (read vs append
  vs finalize-flush vs delete); snapshot `(closed_latest, log_offset)` per
  read; sparse offset index — in-memory for live (built at boot repair,
  maintained per append), persisted rebuildable `{id}/index.bin` written at
  finalize for terminal entities (rebuilt once if missing); boot never
  scans terminal logs.
- **Fetch/confirm:** fetch re-serves pending or creates a new span (page
  cap 64 KiB/200 segments) with an opaque receipt, never advancing
  `confirmed_seq`; confirm CAS-matches
  `(confirmed_seq, pending_from, pending_to, receipt)`; the CLI runs
  fetch → render → stdout flush → confirm in one invocation. Crash before
  flush → span re-serves. Confirm committed but response lost → next read
  serves the next span (safe: content already flushed). Cursors never
  regress.
- **Stateless snapshots:** `--full`/`--range` pin
  `(closed_latest, log_offset)` and loop inside one invocation with an
  opaque continuation token (`--max-bytes` stops early and prints the
  resume command).
- **Self-describing output:** every response names mode, exact span,
  totals, live/terminal status (+`end_source`/`stalled_reason`), and
  follow-up commands (`--range` re-reads any span; `--full` for
  everything). First incremental read from a thread says so; empty
  increments say "no new segments since [N]".

## 7. Conversation references: a text convention (scope reset)

There is no injection, no metadata protocol, no refs table, and no
rendering change. A reference is a line of text:

```
[会议实体 019f… 《Q3 规划会》 — 读取: garyx meeting read 019f…]
```

- **UI "chat about this meeting"** (slice 3): the gallery card's action
  opens a new or existing conversation with the composer **prefilled**
  with that line (user can edit before sending). The message is an
  ordinary user message traveling every existing path untouched. The
  agent reads the id in the text and runs the CLI.
- **Recognition of prior reference** is the cursor row: the thread's first
  `read` creates it; every later `read` is incremental automatically. No
  other component needs to know.
- **Agent discovery** without a reference: `garyx meeting list`.
- A deleted entity makes the read command fail with a clear "entity not
  found"; the stale text line in old conversations is harmless history.

## 8. UI surfaces (slice 3)

- Desktop "Meetings" gallery (left rail): keyset-paged list, live badge,
  refresh on open + 30 s polling while visible (no SSE); actions:
  chat-about (prefill text), admin abort (joining/live), delete
  (terminal).
- iOS: same catalog via stale-while-refresh; logic in `GaryxMobileCore`
  with SwiftPM tests; packaged `xcodebuild` validation.
- No transcript/chip/render_state work anywhere.

## 9. Configuration and deployment prerequisites

- `FeishuAccount.meeting_entities: bool` (`default_true`): disabling stops
  new admissions and unregisters the client; existing entities follow the
  per-state stalled rules.
- `GatewayConfig.meetings: MeetingConfig` — `poll_interval_secs` (30,
  clamp 10–120), `join_retry_window_secs` (300), `read_page_bytes`
  (65536).
- Developer console (blocks slice 2): subscribe the two `vc.bot.*` events,
  publish app version. Scopes already granted and live-verified.
- Fixtures sanitized; no real ids.

## 10. Failure and observability

Lifecycle transitions log info (entity, feishu id, CAS from→to, source);
typed poll/join errors with backoff state; `status_detail`, `end_source`,
`stalled_reason` in `garyx meeting list --json`; boot repair logs
truncated bytes, replayed lines, cursor corrections, tombstone
dispositions.

## 11. Implementation impact map

| Area | Change | Nature |
|---|---|---|
| `garyx-models/src/local_paths.rs` | `default_meetings_dir()` | additive |
| `garyx-models/src/config.rs` | two config fields (serde-default) | additive |
| `garyx-channels/src/meeting_sink.rs` (new) | traits + types + no-op impl | additive |
| `garyx-channels/src/feishu/types.rs` | 2 event structs | additive |
| `garyx-channels/src/feishu/ws.rs` | 2 dispatch branches → sink | additive (2 `else if`) |
| `garyx-channels/src/feishu/client.rs` | `bots_join`/`bots_events` + adapter | additive |
| `garyx-channels/src/plugin.rs`, `feishu.rs` | one constructor dependency | additive |
| `garyx-gateway/src/meetings/` (new) | service, coordinator, log writer/repair, locks, index, routes | new module |
| `garyx-gateway/src/garyx_db/mod.rs` | 2 tables + CRUD | additive |
| `garyx-gateway/src/route_graph.rs` | 7 routes | additive |
| `garyx-gateway/src/composition/*` | service wiring + sink injection | additive |
| `garyx/src/commands/meeting.rs`, `cli.rs`, `main.rs` | CLI subcommand | additive |
| Desktop / iOS (slice 3) | gallery + prefill action | additive |

**Not touched:** bridge, providers, router dispatch, transcripts,
render_state, SSE, thread lifecycle, prompt assembly, queued-input paths.

## 12. Test plan

The subsystem-internal matrix from revision 4 carries over unchanged:
storage/watermark fault injection (torn tails, ckpt-gap re-pull dedup,
empty-tick checkpoints, cross-tick splits, opaque-token resume, fdatasync
ordering); bounded ticks under sustained `has_more`; lifecycle/arbitration
(end>abort, GraceExpired acceleration, 10005-in-finalizing, stalled
matrix, admission-insert failure, reinvite-after-terminal/delete);
registry lifecycle; deletion crash points incl. missing-dir and
tombstone-before-repair ordering; fetch/confirm failure pairs
(pre-flush crash vs confirm-committed-response-lost); write-time split
spans; snapshot continuation across live appends; cold high-seq terminal
reads via persisted index within budget; keyset list stability.

Dropped with the integration surface: stream-input seam tests, allowlist
tests, resolver/spoof/XML tests, reducer/chip contract tests,
thread-delete cascade tests (replaced by: cursor rows of dead threads are
inert; entity delete cascades them).

New: CLI prefill-text convention is exercised end-to-end in slice 3 by a
scripted conversation (send prefilled line → agent runs CLI → increment)
— but this is a product check, not a protocol test; no runtime seam
exists to test.

Scope: `cargo test --all-targets` for `garyx-channels`, `garyx-gateway`,
`garyx-models` (config/paths), and `garyx` (CLI); tier1 fast loop; slice 3
adds SwiftPM + `xcodebuild` + packaged desktop check. (`garyx-bridge` and
`garyx-router` are untouched and need no gate.)

## 13. Delivery slices

1. **Entity core + read protocol** (no Feishu dependency): tables, log
   writer/repair, locks + index, fetch/confirm + snapshot reads, CLI
   list/read/delete, fixture tests. Gate: read-protocol, storage-fault,
   and deletion suites green; hand-seeded entity readable incrementally
   from two thread identities with independent receipt-confirmed cursors.
2. **Ingestion**: sample capture gate → event structs, sink + registry,
   coordinator with intent CAS, bounded ticks, grace drain, recovery,
   admin abort, console subscriptions. Gate: real meeting
   live→finalizing→finalized with correct segments; restarts in every
   non-terminal state resume; lifecycle suite green.
3. **Experience**: galleries + prefill-chat action + polling refresh.
   Gate: card → prefilled message → agent reads increment → answer cites
   new segments, on packaged desktop + simulator.

## 14. Open questions (defaults chosen, non-blocking)

- Retention: manual deletion only.
- One entity per admission (explicit product rule).
- `meeting_activity_v1` ignored in Phase 1.
- If per-turn automatic context injection is ever wanted later, it is a
  separate design with its own review — explicitly out of scope now.
