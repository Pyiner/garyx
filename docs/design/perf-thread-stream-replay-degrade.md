# Perf slice 4 — degrade stale-resume thread streams to a windowed replay

Status: draft for review (perf round, slice 4; measurement done, implementation pending)
Owner: gateway + desktop + iOS
Related: perf round slices 1-3; thread-open floor windowing (initial_user_turns); S5 resumable per-thread stream

## Problem (measured)

"The thread page feels janky" on both Mac and iOS. Rendering itself is
NOT the problem — measured live on the packaged Mac app against the
5,243-record session thread:

| dimension | result |
| --- | --- |
| streaming frame times (20s window) | p50 17 / p95 18 / p99 19ms, 1 long task |
| scroll stress (60 wheel bursts) | p95 18ms, 3/888 frames > 50ms |
| composer typing (200 keys) | per-key p95 17ms |
| open giant thread (warm cursor) | 262–280ms |
| DOM size (windowed rows) | ~10k nodes, 133 rows |

The jank source is the per-thread SSE **replay payload**. A cold
`GET /api/threads/{key}/stream` on that thread returns a first frame of
**20.4MB**: `events` carries **all 5,243 committed records** while
`render_state` is a healthy 0.7MB window.

Why it happens: `build_thread_stream_replay` replays `seq > after_seq`
up to `THREAD_TRANSCRIPT_REPLAY_CAP = 10_000`. The cold-open path is
protected (`replay_scope=initial` → `cold_open_user_turn_window`, ~3
user turns + render floor), but every **resume** with a stale cursor is
not: a client that last opened the thread days ago resumes from its old
frontier and legally replays thousands of records. On a cross-machine
gateway that is seconds of transfer + parse before the thread reacts —
exactly the reported "卡卡的". On iOS the same bytes are also decoded on
the main thread. The gateway pays CPU to serialize these megabytes per
reopen (part of the observed 253% CPU).

Trigger inventory:
- desktop: reopening an active thread whose cached cursor is old
  (machine was away / thread advanced thousands of seqs via bots, tasks,
  other machines);
- iOS: same, plus any reconnect where the persisted frontier is stale;
- any client connecting with `after_seq=0` without declaring
  `replay_scope=initial` (bare integrations).

## Change

### Server (authoritative fix): stale-resume degrades to the initial window

**Opt-in gate (review #TASK-1698 F1).** Degradation only applies to
connections that declare support: the resume request carries
`windowed_resume=1` (query param, sent by desktop and iOS once their
handling ships). Connections without the param — older desktops on a
newer cross-machine gateway, bare integrations — keep today's verbatim
replay unchanged, so no client can be pushed into a
non-contiguous-frame error loop (`ThreadStreamGapError` on desktop,
the resume contiguity guard on iOS). The param is permanent protocol
surface, not a temporary flag: any future client states its capability
the same way.

**Byte-budget trigger (review #TASK-1698 F2).** The degradation
decision is byte-based, matching the acceptance target. The replay
builder already serializes records as it appends; it accumulates the
serialized size and, when the running total exceeds
`THREAD_STREAM_RESUME_REPLAY_BYTE_BUDGET` (proposed: 1 MiB), abandons
the span replay and serves the window instead. A cheap record-count
precheck (span count > `THREAD_TRANSCRIPT_REPLAY_CAP`) short-circuits
without building at all; everything in between is decided by the byte
budget, so a 200-record replay full of multi-KB tool outputs degrades
while a 2,000-record replay of small records may still stream
verbatim.

For a degraded connection the server serves exactly what an
`replay_scope=initial` connection would get: the
`cold_open_user_turn_window` records, `render_state` based on the
window floor, and the frame's existing `window` block
(`floor_seq`, `has_more_above`).

The frame additionally carries `replay: "windowed"` (new field on the
snapshot frame envelope) so opted-in clients know their local committed
cache below the floor is no longer contiguous with this connection and
must be rebuilt from the window instead of appended to.

`Initial` scope behavior, live tailing, SSE ids, and the
within-budget resume path are unchanged. The 10k cap stays as the
absolute upper bound backstop.

### Desktop

Desktop sends `windowed_resume=1` on its per-thread resume connections
and the stream consumer (gateway-mirror transcript lifecycle) treats a
`replay: "windowed"` frame like an initial window: drop the thread's
cached committed records below `window.floor_seq`, adopt the window
records + render_state, keep the earlier-history pagination entry
(`has_more_above`) working. The gap guard (`ThreadStreamGapError` in
`transcript-sync.ts` / `stream.ts`) must recognize the windowed frame
BEFORE contiguity checking — the marker, not seq arithmetic, is what
authorizes the discontinuity. The existing initial/floor handling is
reused; the new code is only the "this resume was degraded" entry
point.

### iOS

iOS sends `windowed_resume=1` once `GatewayStreamFrameProcessor` learns
the frame marker. The processor currently rejects a non-contiguous
first seq on resume connections (`allowsNonContiguousFirstSeq` is only
true for `.initial`); a frame marked `replay: "windowed"` is
self-identifying as a window reset and is accepted through the same
path as an initial window (reset the committed cache to the window,
adopt `render_state`, update both cursors per the frontier rules from
the stream-cursor-frontier design). Ordinary resume frames keep the
contiguity guard — the marker is required, so a real gap still trips
the guard. The planner (`GaryxThreadWindowPlanner`) keeps choosing
resume exactly as today; the server decides when a resume is too
stale. Until the iOS build ships, iOS simply never sends the param and
sees today's behavior.

### Why server-side (vs. client planners choosing initial more often)

The client cannot cheaply know its lag (thread summaries carry no tail
seq), and every client would need the same heuristic. The server knows
the exact span with one cheap count and fixes every client — including
bare `after_seq=0` connections — in one place.

## Validation plan

- Gateway unit: opted-in resume over the byte budget returns the
  windowed frame (records == cold-open window, `replay == "windowed"`,
  floor/has_more_above set); within-budget resume keeps verbatim
  replay; resume WITHOUT `windowed_resume=1` never degrades regardless
  of span (old-client protection); initial scope unchanged. Live-tail
  dedup unchanged. Byte-budget boundary: few large records degrade,
  many small records within budget do not.
- Desktop unit: lifecycle handling of a windowed resume frame drops
  stale cache below floor and renders the window (mirror-contract
  test); earlier-history pagination still loads above the floor.
- iOS SwiftPM: frame processor accepts a `replay:"windowed"` frame on a
  resume connection and resets to the window; ordinary resume frames
  keep the contiguity guard.
- Live before/after on the 5,243-record thread: cold `curl` first
  frame 20.4MB → target < 1MB; desktop stale-cursor reopen and iOS
  reopen walkthrough.

## Measured after implementation

(to fill)
