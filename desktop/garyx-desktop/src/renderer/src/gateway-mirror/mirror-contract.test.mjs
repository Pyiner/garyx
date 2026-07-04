// Contract harness for the GatewayMirror (endgame architecture batch 0).
//
// Locks the four store-mechanics contracts the design depends on:
//   1. getSnapshot reference stability (useSyncExternalStore hard rule)
//   2. per-thread notification isolation
//   3. monotonic render-state acceptance
//   4. committed/render frontier separation (render-only frames must not
//      advance the committed cursor)
//
// Frame sources: reducer cases from test-fixtures/render-layer/
// render-state-cases.json are wrapped into synthesized thread_render_frame
// envelopes (they are records->RenderState reducer cases, not wire frames;
// see the design's batch-0 note).

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { GatewayMirror } from "./mirror.ts";

const here = path.dirname(fileURLToPath(import.meta.url));
const casesPath = path.resolve(
  here,
  "../../../../../../test-fixtures/render-layer/render-state-cases.json",
);
const { cases } = JSON.parse(readFileSync(casesPath, "utf8"));

function committedEventFromRecord(record, threadId) {
  return {
    type: "committed_message",
    runId: record.run_id || "",
    threadId,
    seq: record.seq,
    message: record.message,
  };
}

// Wrap one reducer case into a synthesized wire-shaped frame envelope.
function frameFromCase(reducerCase, threadId) {
  const events = (reducerCase.records || []).map((record) =>
    committedEventFromRecord(record, threadId),
  );
  return {
    type: "thread_render_frame",
    threadId,
    events,
    renderState: reducerCase.expected,
  };
}

function caseWithRecords() {
  const found = cases.find(
    (candidate) =>
      (candidate.records || []).length >= 3 &&
      candidate.expected &&
      typeof candidate.expected.based_on_seq === "number",
  );
  assert.ok(found, "fixture set should contain a case with >=3 records");
  return found;
}

test("getThreadSnapshot returns a stable reference until a change applies", () => {
  const mirror = new GatewayMirror();
  const threadId = "thread::contract-stability";
  const frame = frameFromCase(caseWithRecords(), threadId);

  const empty1 = mirror.getThreadSnapshot(threadId);
  const empty2 = mirror.getThreadSnapshot(threadId);
  assert.equal(empty1, empty2, "unchanged empty snapshot must be same ref");

  mirror.ingest(frame);
  const after1 = mirror.getThreadSnapshot(threadId);
  const after2 = mirror.getThreadSnapshot(threadId);
  assert.notEqual(after1, empty1, "applied frame must produce a new snapshot");
  assert.equal(after1, after2, "unchanged snapshot must be same ref");
  assert.equal(after1.records.length, frame.events.length);

  // Idempotent re-ingest: every seq already cached, same based_on_seq.
  const notified = [];
  mirror.subscribeThread(threadId, () => notified.push(true));
  mirror.ingest(frame);
  const after3 = mirror.getThreadSnapshot(threadId);
  assert.equal(after3, after1, "idempotent re-ingest must not rebuild");
  assert.equal(notified.length, 0, "idempotent re-ingest must not notify");
});

test("per-thread notifications are isolated", () => {
  const mirror = new GatewayMirror();
  const reducerCase = caseWithRecords();
  const frameA = frameFromCase(reducerCase, "thread::contract-a");
  const frameB = frameFromCase(reducerCase, "thread::contract-b");

  let notifiedA = 0;
  let notifiedB = 0;
  mirror.subscribeThread("thread::contract-a", () => (notifiedA += 1));
  const unsubscribeB = mirror.subscribeThread(
    "thread::contract-b",
    () => (notifiedB += 1),
  );

  mirror.ingest(frameA);
  assert.equal(notifiedA, 1, "thread A frame notifies A once");
  assert.equal(notifiedB, 0, "thread A frame must not notify B");

  mirror.ingest(frameB);
  assert.equal(notifiedA, 1);
  assert.equal(notifiedB, 1);

  unsubscribeB();
  mirror.ingest(frameFromCase(caseWithRecords(), "thread::contract-b"));
  assert.equal(notifiedB, 1, "unsubscribed listener must not fire");
});

test("render-state acceptance is monotonic by based_on_seq", () => {
  const mirror = new GatewayMirror();
  const threadId = "thread::contract-monotonic";
  const reducerCase = caseWithRecords();
  const frame = frameFromCase(reducerCase, threadId);
  mirror.ingest(frame);

  const applied = mirror.getThreadSnapshot(threadId);
  const appliedSeq = applied.renderState.based_on_seq;
  assert.ok(appliedSeq > 0);

  const notified = [];
  mirror.subscribeThread(threadId, () => notified.push(true));

  // A stale snapshot-only frame (lower based_on_seq) must be rejected
  // without touching the snapshot or notifying.
  mirror.ingest({
    type: "thread_render_frame",
    threadId,
    events: [],
    renderState: { ...frame.renderState, based_on_seq: appliedSeq - 1 },
  });

  const after = mirror.getThreadSnapshot(threadId);
  assert.equal(after, applied, "stale render must not rebuild the snapshot");
  assert.equal(after.renderState.based_on_seq, appliedSeq);
  assert.equal(notified.length, 0, "stale render must not notify");
});

test("render-only frames advance the render frontier but never the committed cursor", () => {
  const mirror = new GatewayMirror();
  const threadId = "thread::contract-frontier";
  const reducerCase = caseWithRecords();
  const frame = frameFromCase(reducerCase, threadId);
  mirror.ingest(frame);

  const applied = mirror.getThreadSnapshot(threadId);
  const committedSeq = applied.frontier.committedSeq;
  assert.ok(committedSeq > 0);
  assert.equal(
    applied.frontier.renderBasedOnSeq,
    frame.renderState.based_on_seq,
  );

  // Caught-up/replay-cap scenario: a snapshot-only frame whose based_on_seq
  // is ahead of the locally committed tail. The render frontier moves; the
  // committed cursor (safe reconnect afterSeq) must not.
  const aheadSeq = committedSeq + 7;
  mirror.ingest({
    type: "thread_render_frame",
    threadId,
    events: [],
    renderState: { ...frame.renderState, based_on_seq: aheadSeq },
  });

  const after = mirror.getThreadSnapshot(threadId);
  assert.equal(after.frontier.renderBasedOnSeq, aheadSeq);
  assert.equal(
    after.frontier.committedSeq,
    committedSeq,
    "render-only frame must not pollute the committed frontier",
  );
  assert.equal(after.records.length, applied.records.length);
});

test("all fixture reducer cases ingest cleanly through the frame envelope", () => {
  const mirror = new GatewayMirror();
  for (const [index, reducerCase] of cases.entries()) {
    if (!reducerCase.expected || typeof reducerCase.expected.based_on_seq !== "number") {
      continue;
    }
    const threadId = `thread::contract-case-${index}`;
    mirror.ingest(frameFromCase(reducerCase, threadId));
    const snapshot = mirror.getThreadSnapshot(threadId);
    assert.equal(
      snapshot.renderState.based_on_seq,
      reducerCase.expected.based_on_seq,
      `case ${reducerCase.name || index} render cursor`,
    );
    assert.equal(snapshot, mirror.getThreadSnapshot(threadId));
  }
});
