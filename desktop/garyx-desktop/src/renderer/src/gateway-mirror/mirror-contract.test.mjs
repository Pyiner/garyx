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

test("empty-ledger caught-up frame at based_on_seq=0 stores the snapshot", () => {
  const mirror = new GatewayMirror();
  const threadId = "thread::contract-empty-ledger";
  const emptyRender = {
    based_on_seq: 0,
    rows: [],
    tailActivity: "none",
    activeToolGroupId: null,
    progress_locus: "none",
    visibleMessageIds: [],
    filtered_placeholders: [],
  };

  let notified = 0;
  mirror.subscribeThread(threadId, () => (notified += 1));
  mirror.ingest({
    type: "thread_render_frame",
    threadId,
    events: [],
    renderState: emptyRender,
  });

  const snapshot = mirror.getThreadSnapshot(threadId);
  assert.ok(snapshot.renderState, "based_on_seq=0 snapshot must be stored");
  assert.equal(snapshot.renderState.based_on_seq, 0);
  assert.equal(snapshot.frontier.committedSeq, 0);
  assert.equal(notified, 1);

  // Re-delivery of the same empty snapshot stays idempotent.
  mirror.ingest({
    type: "thread_render_frame",
    threadId,
    events: [],
    renderState: emptyRender,
  });
  assert.equal(notified, 1, "same-cursor re-delivery must not notify");
});

test("a frame with new committed events but a stale render applies the events only", () => {
  const mirror = new GatewayMirror();
  const threadId = "thread::contract-mixed";
  const reducerCase = caseWithRecords();
  const frame = frameFromCase(reducerCase, threadId);
  mirror.ingest(frame);
  const applied = mirror.getThreadSnapshot(threadId);
  const appliedRenderSeq = applied.renderState.based_on_seq;
  const topSeq = applied.frontier.committedSeq;

  // A frame carrying one genuinely new committed event but a stale render
  // snapshot: events must apply and advance the committed cursor; the stale
  // render must be rejected without regressing the stored snapshot.
  mirror.ingest({
    type: "thread_render_frame",
    threadId,
    events: [
      {
        type: "committed_message",
        runId: "",
        threadId,
        seq: topSeq + 1,
        message: { id: `late-${topSeq + 1}`, role: "assistant", content: "x" },
      },
    ],
    renderState: { ...frame.renderState, based_on_seq: appliedRenderSeq - 1 },
  });

  const after = mirror.getThreadSnapshot(threadId);
  assert.equal(after.frontier.committedSeq, topSeq + 1, "events must apply");
  assert.equal(
    after.renderState.based_on_seq,
    appliedRenderSeq,
    "stale render must not regress the stored snapshot",
  );

  // Non-finite based_on_seq is rejected outright.
  const beforeRef = mirror.getThreadSnapshot(threadId);
  mirror.ingest({
    type: "thread_render_frame",
    threadId,
    events: [],
    renderState: { ...frame.renderState, based_on_seq: Number.NaN },
  });
  assert.equal(mirror.getThreadSnapshot(threadId), beforeRef);
});

test("root and catalog snapshots are stable and refresh atomically per domain", async () => {
  const desktopState = { threads: [], endpoints: [], configuredBots: [], automations: [] };
  const services = {
    getState: async () => desktopState,
    listCustomAgents: async () => [{ id: "agent-a" }],
    listTeams: async () => {
      throw new Error("teams endpoint down");
    },
    listWorkflowDefinitions: async () => [{ id: "wf-1" }],
  };
  const mirror = new GatewayMirror(services);

  const root1 = mirror.getRootSnapshot();
  assert.equal(mirror.getRootSnapshot(), root1, "empty root snapshot stable");
  const catalog1 = mirror.getCatalogSnapshot();
  assert.equal(mirror.getCatalogSnapshot(), catalog1);

  let rootNotified = 0;
  let catalogNotified = 0;
  mirror.subscribeRoot(() => (rootNotified += 1));
  mirror.subscribeCatalog(() => (catalogNotified += 1));

  const returned = await mirror.refreshDesktopState();
  assert.equal(returned, desktopState);

  const root2 = mirror.getRootSnapshot();
  assert.notEqual(root2, root1);
  assert.equal(root2.desktopState, desktopState);
  assert.equal(mirror.getRootSnapshot(), root2, "refreshed root stable");

  const catalog2 = mirror.getCatalogSnapshot();
  assert.equal(catalog2.agents.length, 1);
  assert.equal(catalog2.teams.length, 0, "failed catalog fetch degrades to []");
  assert.equal(catalog2.workflows.length, 1);
  assert.ok(rootNotified >= 1);
  assert.ok(catalogNotified >= 1);

  // observeConnection touches root only.
  const catalogBefore = mirror.getCatalogSnapshot();
  mirror.observeConnection({ ok: true, bridgeReady: true, gatewayUrl: "http://localhost:1" });
  assert.notEqual(mirror.getRootSnapshot(), root2);
  assert.equal(mirror.getCatalogSnapshot(), catalogBefore, "catalog unaffected");
});

test("observeConnection dedupes shallow-equal statuses from fresh poll objects", () => {
  const mirror = new GatewayMirror();
  let rootNotified = 0;
  mirror.subscribeRoot(() => (rootNotified += 1));

  const status = { ok: true, bridgeReady: true, gatewayUrl: "http://localhost:1" };
  mirror.observeConnection(status);
  assert.equal(rootNotified, 1, "first observation bumps root");
  const root1 = mirror.getRootSnapshot();

  // Healthy poll: a fresh object with identical content must not bump.
  mirror.observeConnection({ ...status });
  assert.equal(rootNotified, 1, "shallow-equal fresh object must not bump");
  assert.equal(mirror.getRootSnapshot(), root1, "snapshot reference stable");

  // A real change bumps again.
  mirror.observeConnection({ ...status, ok: false, error: "down" });
  assert.equal(rootNotified, 2);
  assert.notEqual(mirror.getRootSnapshot(), root1);
});

// ---- Batch 2a-2: dual-run equivalence for the authoritative-apply path ----

import { transcriptWithResolvedActiveRun } from "../../../shared/transcript-sync.ts";
import {
  materializeRemoteTranscript,
  visibleTranscriptMessages,
} from "./transcript-materialize.ts";

// Synthetic TranscriptMessage pool (wire/IPC shape per contracts): the
// render-state fixture records carry ledger-shaped bodies without ids, so
// the authoritative-apply dual-run uses purpose-built messages instead.
function syntheticMessages(count) {
  const pool = [
    // A control record: visibleTranscriptMessages must filter it, so the
    // dual-run also guards the filter step, not just the wiring.
    {
      id: "msg-control",
      role: "system",
      text: "",
      kind: "control",
      internal: true,
    },
  ];
  for (let i = 1; i <= count; i += 1) {
    const role = i % 3 === 0 ? "assistant" : i % 3 === 1 ? "user" : "assistant";
    pool.push({
      id: `msg-${i}`,
      seq: i,
      role,
      text: `synthetic message ${i}`,
      timestamp: `2026-06-19T12:0${i % 10}:00Z`,
    });
  }
  return pool;
}

function transcriptFromCases(threadId, take) {
  const slice = syntheticMessages(take);
  assert.equal(slice.length, take + 1);
  return {
    threadId,
    messages: slice,
    pendingInputs: [],
    threadInfo: null,
  };
}

// Legacy pure path: exactly what applyCanonicalTranscript does to the
// message cache, minus React/message-machine/IPC side effects.
function legacyCanonicalMessages(transcript, existing) {
  const resolved = transcriptWithResolvedActiveRun(transcript);
  const visible = visibleTranscriptMessages(resolved.messages);
  return materializeRemoteTranscript(visible, [...existing]);
}

test("dual-run: mirror authoritative apply matches the legacy pure path", () => {
  const threadId = "thread::dual-run-canonical";
  const first = transcriptFromCases(threadId, 4);
  const second = transcriptFromCases(threadId, 6);

  // Legacy chain: two successive canonical applies over a shared cache.
  const legacyAfterFirst = legacyCanonicalMessages(first, []);
  const legacyAfterSecond = legacyCanonicalMessages(second, legacyAfterFirst);

  // Mirror chain: same inputs through applyAuthoritativeTranscript.
  const mirror = new GatewayMirror();
  let notified = 0;
  mirror.subscribeThread(threadId, () => (notified += 1));

  mirror.applyAuthoritativeTranscript(threadId, first);
  const snapshotFirst = mirror.getThreadSnapshot(threadId);
  assert.deepEqual(snapshotFirst.messages, legacyAfterFirst);
  assert.equal(notified, 1);

  mirror.applyAuthoritativeTranscript(threadId, second);
  const snapshotSecond = mirror.getThreadSnapshot(threadId);
  assert.deepEqual(snapshotSecond.messages, legacyAfterSecond);
  assert.equal(notified, 2);
  assert.notEqual(snapshotSecond, snapshotFirst, "apply must rebuild snapshot");
  assert.equal(snapshotSecond, mirror.getThreadSnapshot(threadId));
});

test("dual-run: threadInfo and pending inputs mirror the resolved transcript", () => {
  const threadId = "thread::dual-run-info";
  const base = transcriptFromCases(threadId, 3);
  const pending = [{ id: "pending-1", content: "queued text" }];
  const transcript = {
    ...base,
    pendingInputs: pending,
    threadInfo: { activeRun: null, workspacePath: "/Users/test/repo" },
  };

  const resolved = transcriptWithResolvedActiveRun(transcript);
  const mirror = new GatewayMirror();
  mirror.applyAuthoritativeTranscript(threadId, transcript);
  const snapshot = mirror.getThreadSnapshot(threadId);

  assert.deepEqual(snapshot.threadInfo, resolved.threadInfo ?? null);
  assert.deepEqual(snapshot.pendingRemoteInputs, resolved.pendingInputs ?? []);
  // The authoritative path must not touch frontiers or verbatim records.
  assert.equal(snapshot.frontier.committedSeq, 0);
  assert.equal(snapshot.records.length, 0);
});
