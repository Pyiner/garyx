// Main-process render-delta reassembler tests (#TASK-1956 batch 2).
//
// Wire fixtures mirror the real gateway (garyx-gateway/src/routes.rs):
// full frames carry `render_state` (with the `rows_hash` chain token as a
// decimal string on delta connections); delta live frames carry
// `render_delta` INSTEAD of `render_state`. Tokens are opaque to the
// client — pure equality, never hashing — so fixtures may use arbitrary
// strings.
import test from "node:test";
import assert from "node:assert/strict";

import { ThreadStreamGapError, streamThreadEvents } from "./stream.ts";

const THREAD = "thread::delta";

function committedEvent(seq, text) {
  return {
    type: "committed_message",
    thread_id: THREAD,
    run_id: `run-${THREAD}`,
    seq,
    message: {
      role: "assistant",
      content: text,
      text,
      timestamp: "2026-07-09T12:00:00Z",
    },
  };
}

function userTurnRow(id, revision) {
  return {
    kind: "user_turn",
    id,
    user: { id: `origin:${id}`, seq: 1, role: "user" },
    activity: [
      {
        kind: "assistant_reply",
        id: `assistant_reply:${id}:rev${revision}`,
        message: { id: "seq:2", seq: 2, role: "assistant" },
        streaming: false,
      },
    ],
    started_at: null,
    finished_at: null,
  };
}

function renderStateFixture(basedOnSeq, rows, rowsHash, extra = {}) {
  return {
    based_on_seq: basedOnSeq,
    rows,
    tailActivity: "none",
    activeToolGroupId: null,
    progress_locus: "none",
    visibleMessageIds: [],
    filtered_placeholders: [],
    ...(rowsHash === null ? {} : { rows_hash: rowsHash }),
    ...extra,
  };
}

function deltaFixture(overrides = {}) {
  return {
    from_seq: 5,
    from_rows_hash: "1001",
    based_on_seq: 6,
    rows_hash: "1002",
    row_order: [],
    upsert_rows: [],
    tailActivity: "assistant_streaming",
    activeToolGroupId: null,
    progress_locus: "tail",
    filtered_placeholders: [],
    ...overrides,
  };
}

function fullFrame(sseId, events, renderState) {
  const payload = JSON.stringify({
    type: "thread_render_frame",
    thread_id: THREAD,
    events,
    render_state: renderState,
  });
  return `id: ${sseId}\ndata: ${payload}\n\n`;
}

function deltaFrame(sseId, events, delta) {
  const payload = JSON.stringify({
    type: "thread_render_frame",
    thread_id: THREAD,
    events,
    render_delta: delta,
  });
  return `id: ${sseId}\ndata: ${payload}\n\n`;
}

function sseResponse(...frames) {
  return new Response(
    new ReadableStream({
      start(controller) {
        for (const frame of frames) {
          controller.enqueue(new TextEncoder().encode(frame));
        }
        controller.close();
      },
    }),
    { status: 200, statusText: "OK" },
  );
}

async function collectStream(frames, options = {}) {
  const originalFetch = globalThis.fetch;
  const events = [];
  const committedSeqs = [];
  globalThis.fetch = async () => sseResponse(...frames);
  let error = null;
  try {
    await streamThreadEvents(
      { gatewayUrl: "http://127.0.0.1:31337", gatewayAuthToken: "" },
      THREAD,
      (event) => events.push(event),
      undefined,
      {
        afterSeq: 4,
        onCommittedSeq: (seq) => committedSeqs.push(seq),
        ...options,
      },
    );
  } catch (caught) {
    error = caught;
  } finally {
    globalThis.fetch = originalFetch;
  }
  return { events, committedSeqs, error };
}

function seedFrame() {
  // Connection seed: full frame at seq 5 with rows A@1 + B@1, token "1001".
  return fullFrame(
    5,
    [committedEvent(5, "seed")],
    renderStateFixture(5, [userTurnRow("row-a", 1), userTurnRow("row-b", 1)], "1001"),
  );
}

test("delta frame reassembles a full render_state from the held snapshot", async () => {
  const { events, committedSeqs, error } = await collectStream([
    seedFrame(),
    deltaFrame(
      6,
      [committedEvent(6, "delta")],
      deltaFixture({
        row_order: ["row-a", "row-b", "row-c"],
        upsert_rows: [userTurnRow("row-b", 2), userTurnRow("row-c", 1)],
      }),
    ),
  ]);

  assert.equal(error, null);
  assert.equal(events.length, 2);
  const reassembled = events[1];
  assert.equal(reassembled.type, "thread_render_frame");
  assert.equal(reassembled.events.length, 1);
  assert.equal(reassembled.events[0].seq, 6);
  // Rebuilt in row_order: upsert body wins (row-b rev2, row-c new), the
  // untouched row rides over from the held snapshot (row-a rev1).
  assert.deepEqual(reassembled.renderState.rows, [
    userTurnRow("row-a", 1),
    userTurnRow("row-b", 2),
    userTurnRow("row-c", 1),
  ]);
  assert.equal(reassembled.renderState.based_on_seq, 6);
  assert.equal(reassembled.renderState.rows_hash, "1002");
  // Scalars replaced wholesale from the delta.
  assert.equal(reassembled.renderState.tailActivity, "assistant_streaming");
  assert.equal(reassembled.renderState.progress_locus, "tail");
  assert.deepEqual(reassembled.renderState.visibleMessageIds, []);
  assert.deepEqual(committedSeqs, [5, 6]);
});

test("delta chain stays live across consecutive delta frames", async () => {
  const { events, error } = await collectStream([
    seedFrame(),
    deltaFrame(
      6,
      [committedEvent(6, "one")],
      deltaFixture({
        row_order: ["row-a", "row-b"],
        upsert_rows: [userTurnRow("row-b", 2)],
      }),
    ),
    // Anchored on the PREVIOUS DELTA's token — proves the accepted
    // delta's rows_hash became the stored chain token.
    deltaFrame(
      7,
      [committedEvent(7, "two")],
      deltaFixture({
        from_seq: 6,
        from_rows_hash: "1002",
        based_on_seq: 7,
        rows_hash: "1003",
        row_order: ["row-a", "row-b", "row-d"],
        upsert_rows: [userTurnRow("row-d", 1)],
      }),
    ),
  ]);

  assert.equal(error, null);
  assert.equal(events.length, 3);
  assert.deepEqual(events[2].renderState.rows, [
    userTurnRow("row-a", 1),
    userTurnRow("row-b", 2),
    userTurnRow("row-d", 1),
  ]);
  assert.equal(events[2].renderState.rows_hash, "1003");
});

test("delta window and rateLimit scalars ride along whole", async () => {
  const floors = [];
  const { events, error } = await collectStream(
    [
      seedFrame(),
      deltaFrame(
        6,
        [committedEvent(6, "windowed")],
        deltaFixture({
          row_order: ["row-a", "row-b"],
          upsert_rows: [],
          window: { floor_seq: 4711, has_more_above: true },
          rateLimit: { provider: "claude", willAutoResend: true },
        }),
      ),
    ],
    { onWindowFloor: (floorSeq) => floors.push(floorSeq) },
  );

  assert.equal(error, null);
  assert.deepEqual(events[1].renderState.window, {
    floor_seq: 4711,
    has_more_above: true,
  });
  assert.deepEqual(events[1].renderState.rateLimit, {
    provider: "claude",
    willAutoResend: true,
  });
  assert.deepEqual(floors, [4711]);
  // Absent-on-the-wire scalars stay absent after reassembly, mirroring the
  // full frame's serde skip_serializing_if shape.
  assert.equal("window" in events[0].renderState, false);
  assert.equal("rateLimit" in events[1].events[0], false);
});

test("a full frame mid-stream unconditionally reseeds the chain", async () => {
  const { events, error } = await collectStream([
    seedFrame(),
    deltaFrame(
      6,
      [committedEvent(6, "one")],
      deltaFixture({
        row_order: ["row-a", "row-b"],
        upsert_rows: [userTurnRow("row-b", 2)],
      }),
    ),
    // Snapshot-only full frame (events: []) with a fresh base: replay,
    // snapshot-only, and same-seq-reseed frames all take this path.
    fullFrame(
      6,
      [],
      renderStateFixture(6, [userTurnRow("row-x", 1)], "9001"),
    ),
    // The next delta must anchor on the reseeded token and rows — the
    // pre-reseed chain ("1002") is dead.
    deltaFrame(
      7,
      [committedEvent(7, "after reseed")],
      deltaFixture({
        from_seq: 6,
        from_rows_hash: "9001",
        based_on_seq: 7,
        rows_hash: "9002",
        row_order: ["row-x", "row-y"],
        upsert_rows: [userTurnRow("row-y", 1)],
      }),
    ),
  ]);

  assert.equal(error, null);
  assert.equal(events.length, 4);
  assert.deepEqual(events[3].renderState.rows, [
    userTurnRow("row-x", 1),
    userTurnRow("row-y", 1),
  ]);
  assert.equal(events[3].renderState.rows_hash, "9002");
});

test("delta from_seq mismatch discards the frame and gap-reconnects", async () => {
  const { events, error } = await collectStream([
    seedFrame(),
    deltaFrame(
      6,
      [committedEvent(6, "stale base")],
      deltaFixture({
        from_seq: 4,
        row_order: ["row-a", "row-b"],
        upsert_rows: [],
      }),
    ),
  ]);

  assert.ok(error instanceof ThreadStreamGapError);
  assert.equal(error.resumeAfterSeq, 5);
  assert.match(error.message, /from_seq 4 does not match held snapshot seq 5/);
  // The violating frame is discarded atomically: its committed events were
  // never forwarded downstream.
  assert.equal(events.length, 1);
});

test("delta chain-token mismatch discards the frame and gap-reconnects", async () => {
  const { events, error } = await collectStream([
    seedFrame(),
    deltaFrame(
      6,
      [committedEvent(6, "drifted base")],
      deltaFixture({
        from_rows_hash: "6666",
        row_order: ["row-a", "row-b"],
        upsert_rows: [],
      }),
    ),
  ]);

  assert.ok(error instanceof ThreadStreamGapError);
  assert.equal(error.resumeAfterSeq, 5);
  assert.match(error.message, /from_rows_hash does not match held rows-hash token/);
  assert.equal(events.length, 1);
});

test("row_order id missing from upserts and held snapshot gap-reconnects", async () => {
  const { events, error } = await collectStream([
    seedFrame(),
    deltaFrame(
      6,
      [committedEvent(6, "ghost row")],
      deltaFixture({
        row_order: ["row-a", "row-ghost"],
        upsert_rows: [],
      }),
    ),
  ]);

  assert.ok(error instanceof ThreadStreamGapError);
  assert.match(
    error.message,
    /row id row-ghost missing from upsert rows and held snapshot/,
  );
  assert.equal(events.length, 1);
});

test("upsert row absent from row_order gap-reconnects", async () => {
  const { error } = await collectStream([
    seedFrame(),
    deltaFrame(
      6,
      [committedEvent(6, "stray upsert")],
      deltaFixture({
        row_order: ["row-a", "row-b"],
        upsert_rows: [userTurnRow("row-stray", 1)],
      }),
    ),
  ]);

  assert.ok(error instanceof ThreadStreamGapError);
  assert.match(error.message, /upsert row id row-stray is absent from row_order/);
});

test("duplicate upsert row id gap-reconnects", async () => {
  const { error } = await collectStream([
    seedFrame(),
    deltaFrame(
      6,
      [committedEvent(6, "dup upsert")],
      deltaFixture({
        row_order: ["row-a", "row-b"],
        upsert_rows: [userTurnRow("row-b", 2), userTurnRow("row-b", 3)],
      }),
    ),
  ]);

  assert.ok(error instanceof ThreadStreamGapError);
  assert.match(error.message, /upsert row id row-b appears more than once/);
});

test("a delta with no held snapshot gap-reconnects", async () => {
  const { events, error } = await collectStream([
    deltaFrame(
      5,
      [committedEvent(5, "orphan delta")],
      deltaFixture({ row_order: [], upsert_rows: [] }),
    ),
  ]);

  assert.ok(error instanceof ThreadStreamGapError);
  assert.equal(error.resumeAfterSeq, 4);
  assert.equal(events.length, 0);
});

test("a held snapshot without a chain token cannot anchor a delta", async () => {
  // A full frame missing rows_hash never happens on a declared delta
  // connection, but if it does the chain must fail closed, not open.
  const { error } = await collectStream([
    fullFrame(
      5,
      [committedEvent(5, "tokenless seed")],
      renderStateFixture(5, [userTurnRow("row-a", 1)], null),
    ),
    deltaFrame(
      6,
      [committedEvent(6, "unanchored")],
      deltaFixture({ row_order: ["row-a"], upsert_rows: [] }),
    ),
  ]);

  assert.ok(error instanceof ThreadStreamGapError);
  assert.match(error.message, /from_rows_hash does not match held rows-hash token/);
});

test("malformed render_delta payload gap-reconnects instead of being skipped", async () => {
  // Silently skipping a bad delta frame would lose its committed events
  // and mis-render until the next frame; it must take the gap path.
  const { error } = await collectStream([
    seedFrame(),
    deltaFrame(6, [committedEvent(6, "malformed")], { from_seq: 5 }),
  ]);

  assert.ok(error instanceof ThreadStreamGapError);
  assert.match(error.message, /render delta frame is malformed/);
});

test("guard: renderer-facing events always carry a full render_state and never a delta", async () => {
  const { events, error } = await collectStream([
    seedFrame(),
    deltaFrame(
      6,
      [committedEvent(6, "delta")],
      deltaFixture({
        row_order: ["row-a", "row-b", "row-c"],
        upsert_rows: [userTurnRow("row-b", 2), userTurnRow("row-c", 1)],
      }),
    ),
  ]);

  assert.equal(error, null);
  assert.equal(events.length, 2);
  for (const event of events) {
    assert.equal(event.type, "thread_render_frame");
    // Full snapshot: rows materialized, never a patch.
    assert.ok(Array.isArray(event.renderState.rows));
    assert.equal(typeof event.renderState.based_on_seq, "number");
    for (const row of event.renderState.rows) {
      assert.equal(row.kind, "user_turn");
      assert.equal(typeof row.id, "string");
    }
    // The delta encoding never leaks past the transport layer: the
    // desktop event contract is byte-identical to a full-frame connection.
    assert.equal("renderDelta" in event, false);
    assert.equal("render_delta" in event, false);
    assert.equal("render_state" in event, false);
  }
  // The reassembled frame is indistinguishable from a server full frame
  // carrying the same snapshot.
  assert.deepEqual(events[1].renderState.rows, [
    userTurnRow("row-a", 1),
    userTurnRow("row-b", 2),
    userTurnRow("row-c", 1),
  ]);
});
