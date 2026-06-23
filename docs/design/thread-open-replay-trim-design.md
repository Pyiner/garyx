# Thread Open Replay Trim Design

Status: implementation design for thread-open replay trimming and mobile render
alignment.

All examples use synthetic placeholders such as `Test User`,
`thread::synthetic`, and `/Users/test`.

## Problem

Mobile opens an existing thread through per-thread SSE:

```text
GET /api/threads/{thread_id}/stream?after_seq=<cursor>
```

Today the stream replays by sequence cursor and derives `render_state` from the
full committed ledger. Mobile may only have message bodies for the newest
bounded user-turn window. A full render snapshot plus a partial body cache can
produce three symptoms:

- user turns without a user bubble
- collapsed tool rows whose referenced bodies are missing
- assistant content that appears missing until more bodies arrive

The same timing family can briefly show tail thinking before the first assistant
token. The reducer state is correct; the visible transition needs presentation
smoothing.

## Core Decisions

1. Cold-open trimming is explicit. The stream accepts
   `replay_scope=resume|initial`, defaulting to `resume`. Only
   `replay_scope=initial&initial_user_turns=N` asks the server to choose a
   newest-user-turn window. A request with `Last-Event-ID` is always resume and
   ignores initial-window parameters.
2. The render lower bound is called `render_floor`. It selects the committed
   sequence floor used to derive `render_state.rows`. Cold open may bootstrap it
   from `initial_user_turns`; reconnects and scroll expansion send it
   explicitly.
3. Initial-window replay and resume catch-up are separate record selection
   paths. An intentional initial `floor_seq > after_seq + 1` must never enter
   the resume gap-fill / forward-page branch.
4. Windowed snapshots derive `run_state` from the full committed prefix, but
   derive rows from only `[render_floor..tail]`. `based_on_seq` remains the
   committed window tail and still matches the SSE frame id.
5. Initial frames carry the full committed bodies for the selected window. They
   are not snapshot-only unless the thread has no committed records. `events`
   and `render_state` use the same window records.
6. Scroll-up remains server-render-state first: HTTP `before_index` loads older
   bodies, mobile lowers `render_floor`, then reconnects SSE to get an expanded
   server-derived `render_state`. HTTP returning expanded render state is a
   documented fallback only if reconnect jank is proven.
7. Thinking smoothing is presentation-only: delayed display, then crossfade.
   There is no minimum display duration, because real assistant text should not
   be held behind stale thinking chrome.
8. Mobile's existing sequence planner already supports a window reset:
   `decide(incomingSeq: W, connectionLastSeq: 0) == .apply` for `W > 1`.
   This behavior is guarded by tests; the planner logic should not change.
9. Channel `committed_replay.rs` lag-frontier rules are out of scope. This
   design changes per-thread SSE only.
10. Everything is opt-in. Omitted params mean `render_floor=0`, full render
    snapshots, and current behavior. `RenderSnapshot.window` is additive and
    omitted for full snapshots.

## Server Contract

### Stream Params

`ThreadStreamParams` gains optional fields:

```text
after_seq: u64
replay_scope: Option<ThreadStreamReplayScope> // resume | initial, default resume
initial_user_turns: Option<usize>
render_floor: Option<u64>
```

Effective `after_seq` still comes from `Last-Event-ID` when present, otherwise
the query param. If `Last-Event-ID` exists, effective replay scope is always
`resume`.

The connection-level render floor is selected once:

```text
if render_floor is present:
  floor = render_floor
else if replay_scope == initial and initial_user_turns is present:
  floor = newest N user-turn window floor
else:
  floor = 0
```

### Render Window Metadata

`RenderSnapshot` gains:

```rust
#[serde(rename = "window", default, skip_serializing_if = "Option::is_none")]
pub window: Option<RenderWindow>

pub struct RenderWindow {
    pub floor_seq: u64,
    pub has_more_above: bool,
}
```

Full snapshots use `window: None` so old golden output and desktop behavior stay
unchanged.

### Windowed Snapshot Derivation

`render_snapshot_in_window(thread_id, floor_seq, based_on_seq)` reads committed
records up to `based_on_seq`, derives `TranscriptRunState` from the full prefix,
then derives `RenderSnapshot.rows` from only records whose `seq >= floor_seq`.

This preserves busy, rate-limit, active tool group, and tail thinking state
without showing rows outside the declared body window.

### Initial Replay

`cold_open_user_turn_window(thread_id, initial_user_turns, cap)` selects the
newest N human user turns using the existing `is_user_query_message` semantics.
It returns:

- ordered window records
- `floor_seq`
- `has_more_above`

The initial replay builder uses those records directly for the frame `events`
and uses the same floor/tail for `render_state`. This path is intentionally not
fed through resume gap-fill logic.

### Resume, Snapshot-Only, And Live

Normal resume keeps the existing event cursor semantics:

- replay committed records with `seq > after_seq`
- use the existing cap and forward-page gap-fill behavior
- terminate on non-contiguous live seq so the client reconnects

If the connection has `render_floor > 0`, snapshot-only and live frames derive
`render_state` with the connection floor. Event delivery is still controlled
only by `after_seq` and live sequence checks.

## Mobile Contract

### Cold Open And Reconnect

Selected-thread cold open sends:

```text
after_seq=0&replay_scope=initial&initial_user_turns=1
```

After the first accepted render frame, mobile stores:

- tail cursor: `render_state.based_on_seq`
- current render floor: `render_state.window.floor_seq` when present, otherwise
  `0`

Reconnects send:

```text
after_seq=<tail>&render_floor=<known floor>
```

They omit `replay_scope=initial`.

### Scroll-Up Expansion

When the user reaches the top of the displayed window:

1. HTTP loads older bodies with `before_index = render_floor - 1`.
2. The body cache is prepended using existing transcript cache semantics.
3. Mobile lowers its render floor to the oldest newly loaded record seq.
4. Mobile reconnects SSE with the unchanged tail and lowered `render_floor`.
5. The server sends a snapshot-only expanded `render_state` for
   `[new_floor..tail]`.

Mobile does not concatenate render rows or recompute tool/user grouping.

### Defensive Rendering

Mobile keeps the mapper dumb but improves incomplete-body presentation:

- unresolved assistant refs continue to use the existing assistant placeholder
- unresolved user refs get a symmetric loading user placeholder with the same
  committed identity (`history:<seq - 1>`) so the row upgrades in place
- before the first frame, and while expected bodies are still in flight, the UI
  shows skeleton/loading chrome rather than half-materialized transcript rows

These are presentation fallbacks, not grouping or tail-state derivation.

### Thinking Smoothing

`tailActivity == .thinking` is still server-owned. The UI transforms that
boolean through a small presentation model:

- show only after roughly 180-250ms of continuous thinking
- hide when the state leaves thinking
- crossfade over roughly 150ms when motion is allowed
- reduce motion disables animation but keeps delayed display

There is no minimum display duration.

## Gapless Argument

Trimming resets the visible render window, not the event stream.

Example with three turns, tail `9`, newest turn starting at `7`:

1. Initial open sends events `[7,8,9]`, rows `[7..9]`, and
   `based_on_seq=9`.
2. Mobile advances the stream cursor to `9`.
3. Resume from `after_seq=9&render_floor=7` replays only records committed after
   `9`, or emits a snapshot-only frame if caught up.
4. Older records `[1..6]` are outside the opening render window and are loaded
   by HTTP scroll-up pagination, not by forward SSE replay.

Therefore SSE remains gapless for forward delivery. Records below
`render_floor` are historical pages, not missing deltas.

## Phased Implementation

The commit order is:

1. **E: mobile thinking smoothing.** Presentation-only delay and crossfade with
   injected-clock tests.
2. **A: server window plumbing.** `RenderWindow`, windowed snapshot derivation,
   params parsing, and floor propagation through replay/snapshot/live. Omitted
   params stay full-history.
3. **B: server initial replay trim.** User-turn window selection and initial
   frames carrying window bodies. Initial and resume record paths stay separate.
4. **D: mobile rendering fallbacks.** User placeholders and skeleton/loading
   state for incomplete initial materialization.
5. **C: mobile behavior switch.** Send initial params, store `window.floor_seq`,
   reconnect with `render_floor`, and scroll-up by lowering floor then
   reconnecting.

## Required Tests

### Rust

- no-floor regression: existing caught-up snapshot-only behavior remains full
  render state and omits `window`
- `render_floor` snapshot-only frame has empty events and windowed rows
- initial replay with one user turn trims rows and carries all referenced bodies
- `Last-Event-ID` ignores `replay_scope=initial`
- initial floor greater than `after_seq + 1` does not enter resume gap-fill
- live frames after initial/windowed connection stay gapless and keep the floor
- windowed render uses full-prefix run state for busy, rate limit, and active
  tool group correctness

### SwiftPM / GaryxMobileCore

- `GaryxStreamSeqPlanner.decide(W > 1, 0) == .apply`
- full render snapshot plus one-turn body cache reproduces the orphan-row risk
- windowed render snapshot plus one-turn body cache maps to one complete turn
- unresolved user refs produce loading user placeholders
- window planner sequence: cold open, accepted frame, reconnect, scroll-up
  expansion, reconnect
- thinking debounce: short thinking never appears; long thinking appears after
  the threshold and hides when assistant text arrives

### Build Validation

If new iOS files are added, run `xcodegen generate`, commit project changes, run
`swift test`, and build the iOS app target with `xcodebuild`.

## Compatibility And Rollback

Old clients omit all new params and keep current behavior. New servers emit
`window` only for windowed snapshots. Client rollback is simply to stop sending
`replay_scope`, `initial_user_turns`, and `render_floor`.

There are no persistent schema changes. Server A/B phases are additive and
opt-in; mobile D/E can be reverted independently from the replay trim.
