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

// ---- Batch 3a: dispatch-machine storage domain ----

test("machine dispatch commits through the shared reducer with useReducer bail-out semantics", () => {
  const mirror = new GatewayMirror();
  let machineNotified = 0;
  let threadNotified = 0;
  mirror.subscribeMachine(() => (machineNotified += 1));
  mirror.subscribeThread("thread::machine-isolation", () => (threadNotified += 1));

  const initial = mirror.getMachineState();
  assert.equal(mirror.getMachineState(), initial, "state reference stable");

  const intent = {
    intentId: "intent-machine-1",
    threadId: "thread::machine-isolation",
    state: "queued_local",
    dispatchMode: "sync_send",
    responseText: "",
  };
  const afterCreate = mirror.dispatchMachineAction({
    type: "intent/created",
    intent,
    enqueue: true,
  });
  assert.notEqual(afterCreate, initial, "real action commits a new state");
  assert.equal(mirror.getMachineState(), afterCreate, "dispatch returns the committed state");
  assert.equal(afterCreate.intentsById[intent.intentId].intentId, intent.intentId);
  assert.equal(machineNotified, 1, "machine subscribers notified once");
  assert.equal(threadNotified, 0, "machine dispatch must not notify thread subscribers");

  // Reducer bail-out (unknown intent id): same reference, no notify —
  // matching React useReducer's Object.is bail-out the AppShell relied on.
  const afterNoop = mirror.dispatchMachineAction({
    type: "intent/request-dispatch",
    intentId: "intent-does-not-exist",
  });
  assert.equal(afterNoop, afterCreate, "no-op action returns the same reference");
  assert.equal(machineNotified, 1, "no-op action must not notify");

  // Transcript commits must not notify machine subscribers.
  mirror.applyRemoteTranscript("thread::machine-isolation", {
    threadId: "thread::machine-isolation",
    remoteFound: true,
    messages: [wireMessage(1, "user", "hello")],
    pendingInputs: [],
    threadInfo: null,
    pageInfo: fullPageInfo({ totalMessages: 1, committedMessages: 1, returnedMessages: 1, endIndex: 1 }),
  });
  assert.equal(machineNotified, 1, "transcript commit must not notify machine");
  assert.equal(threadNotified, 1, "transcript commit notifies the thread");
});

// ---- Batch 3b: local-write bridge into the mirror message cache ----

test("syncThreadUiMessages bridges local rows and applyRemote preserves them via the intent lookup", () => {
  const threadId = "thread::local-bridge";
  const intent = {
    intentId: "intent-bridge-1",
    threadId,
    state: "awaiting_response",
    dispatchMode: "sync_send",
    responseText: "",
  };
  const intents = { [intent.intentId]: intent };
  const mirror = new GatewayMirror({
    getState: async () => ({}),
    listCustomAgents: async () => [],
    listTeams: async () => [],
    listWorkflowDefinitions: async () => [],
    getThreadHistory: async () => {
      throw new Error("unused");
    },
    intentForId: (id) => intents[id] || null,
  });

  let notified = 0;
  mirror.subscribeThread(threadId, () => (notified += 1));

  // Bridge a locally-written array (optimistic user row appended by the
  // legacy dispatch path).
  const optimisticUser = {
    id: "local-user-bridge",
    role: "user",
    text: "optimistic send",
    localState: "pending",
    intentId: intent.intentId,
  };
  mirror.syncThreadUiMessages(threadId, [optimisticUser]);
  assert.equal(notified, 1, "bridge write commits and notifies once");
  assert.deepEqual(mirror.getThreadSnapshot(threadId).messages, [optimisticUser]);

  // A remote apply WITHOUT the echo must preserve the bridged local row
  // through the mirror's own merge (intentForId services seam).
  mirror.applyRemoteTranscript(threadId, {
    threadId,
    remoteFound: true,
    messages: [wireMessage(1, "user", "earlier"), wireMessage(2, "assistant", "earlier reply")],
    pendingInputs: [],
    threadInfo: null,
    pageInfo: fullPageInfo(),
  });
  const preserved = mirror.getThreadSnapshot(threadId).messages;
  assert.ok(
    preserved.some((entry) => entry.id === optimisticUser.id),
    "unechoed optimistic row survives the mirror-side remote merge",
  );

  // A remote apply WITH the origin-id echo drops the local copy.
  mirror.applyRemoteTranscript(threadId, {
    threadId,
    remoteFound: true,
    messages: [
      wireMessage(1, "user", "earlier"),
      wireMessage(2, "assistant", "earlier reply"),
      {
        id: userMessageIdForOrigin(intent.intentId),
        role: "user",
        text: "optimistic send",
        timestamp: "2026-06-19T12:24:00Z",
      },
    ],
    pendingInputs: [],
    threadInfo: null,
    pageInfo: fullPageInfo({ totalMessages: 3, committedMessages: 3, returnedMessages: 3, endIndex: 3 }),
  });
  const deduped = mirror.getThreadSnapshot(threadId).messages;
  assert.ok(
    !deduped.some((entry) => entry.id === optimisticUser.id),
    "echoed optimistic row is replaced by the remote copy in the mirror",
  );

  // Deleted-key bridge (3b review follow-up): a legacy map that drops a
  // thread key syncs an empty array so the mirror holds no stale rows.
  const before = notified;
  mirror.syncThreadUiMessages(threadId, []);
  assert.deepEqual(mirror.getThreadSnapshot(threadId).messages, []);
  assert.equal(notified, before + 1, "empty bridge still commits once");
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

// ---- Batch 2a-2 part 2: dual-run for the committed-stream mapping, the ----
// ---- remote apply, and the older-history page load ----

import { transcriptRewriteAction } from "../../../shared/transcript-sync.ts";
import {
  committedMessageForwardPage,
  mergeRemotePaginationState,
  mergeRemoteTranscriptWithLocal,
  paginationStateFromTranscript,
  userMessageIdForOrigin,
  THREAD_HISTORY_PAGE_SIZE,
  THREAD_HISTORY_USER_QUERY_LIMIT,
} from "./transcript-materialize.ts";

// Wire-shaped transcript message whose id carries the history index the
// shared forward merge keys on (`transcriptMessageIndex` reads the `:N`
// suffix).
function wireMessage(index, role, text) {
  return {
    id: `seq:${index}`,
    role,
    text,
    timestamp: `2026-06-19T12:2${index % 10}:00Z`,
  };
}

function committedEvent(threadId, seq, message) {
  return { type: "committed_message", runId: "run-1", threadId, seq, message };
}

function fullPageInfo(overrides = {}) {
  return {
    totalMessages: 2,
    committedMessages: 2,
    returnedMessages: 2,
    startIndex: 1,
    endIndex: 2,
    hasMoreBefore: false,
    nextBeforeIndex: null,
    hasMoreAfter: false,
    nextAfterIndex: null,
    limit: 100,
    userQueryLimit: 10,
    ...overrides,
  };
}

// Legacy pure composition: exactly the state transformation the hook's
// applyRemoteTranscript performs on its transcript-domain slices, minus
// React/machine/IPC side effects. Kept as an independent fold in the test
// so the mirror's composition (order + inputs) is compared against the
// hook's, not against itself.
function legacyRemoteApply(state, transcript, intentForId = () => null) {
  const resolved = transcriptWithResolvedActiveRun(transcript);
  const pagination = mergeRemotePaginationState(
    state.pagination,
    paginationStateFromTranscript(resolved),
    [...state.messages],
  );
  const visible = visibleTranscriptMessages(resolved.messages);
  const merged = mergeRemoteTranscriptWithLocal(visible, [...state.messages], {
    activeRunLiveRows: Boolean(resolved.threadInfo?.activeRun),
    preserveRemoteBeforeIndex: resolved.pageInfo?.startIndex ?? null,
    threadRunActive: Boolean(resolved.threadInfo?.activeRun),
    intentForId,
  });
  return {
    snapshot: resolved,
    pagination,
    threadInfo: resolved.threadInfo ?? null,
    pendingInputs: resolved.pendingInputs ?? [],
    messages: merged,
  };
}

// Legacy pure composition of applyCommittedThreadMessage's transcript-domain
// path: rewrite check, forward-page fold, then the remote apply.
function legacyCommittedApply(state, event, intentForId = () => null) {
  if (transcriptRewriteAction(event.message) === "refetch_authoritative") {
    return { ...state, refetchRequested: true };
  }
  return legacyRemoteApply(
    state,
    committedMessageForwardPage(state.snapshot, event),
    intentForId,
  );
}

const emptyLegacyState = {
  snapshot: null,
  pagination: null,
  threadInfo: null,
  pendingInputs: [],
  messages: [],
};

function assertThreadMatchesLegacy(mirror, threadId, legacy) {
  const snapshot = mirror.getThreadSnapshot(threadId);
  assert.deepEqual(snapshot.messages, legacy.messages);
  assert.deepEqual(snapshot.threadInfo, legacy.threadInfo);
  assert.deepEqual(snapshot.pendingRemoteInputs, legacy.pendingInputs);
  assert.deepEqual(snapshot.historyPagination, legacy.pagination);
}

test("dual-run: remote apply plus committed-stream frames match the legacy pure path", () => {
  const threadId = "thread::dual-run-committed";
  const fullTranscript = {
    threadId,
    remoteFound: true,
    messages: [wireMessage(1, "user", "hello"), wireMessage(2, "assistant", "hi")],
    pendingInputs: [],
    threadInfo: null,
    pageInfo: fullPageInfo(),
  };

  // Legacy chain: full fetch apply, then two committed stream records.
  let legacy = legacyRemoteApply(emptyLegacyState, fullTranscript);
  const committed3 = committedEvent(
    threadId,
    3,
    wireMessage(3, "user", "follow-up"),
  );
  const committed4 = committedEvent(
    threadId,
    4,
    wireMessage(4, "assistant", "answer"),
  );

  // Mirror chain: same inputs through the public methods.
  const mirror = new GatewayMirror();
  mirror.applyRemoteTranscript(threadId, fullTranscript);
  assertThreadMatchesLegacy(mirror, threadId, legacy);

  legacy = legacyCommittedApply(legacy, committed3);
  mirror.ingest({
    type: "thread_render_frame",
    threadId,
    events: [committed3],
    renderState: {
      based_on_seq: 3,
      rows: [],
      tailActivity: "none",
      activeToolGroupId: null,
      progress_locus: "none",
      visibleMessageIds: [],
      filtered_placeholders: [],
    },
  });
  assertThreadMatchesLegacy(mirror, threadId, legacy);

  legacy = legacyCommittedApply(legacy, committed4);
  mirror.ingest(committed4);
  assertThreadMatchesLegacy(mirror, threadId, legacy);

  // The verbatim record ledger and frontier advanced alongside the mapping.
  const snapshot = mirror.getThreadSnapshot(threadId);
  assert.equal(snapshot.records.length, 2);
  assert.equal(snapshot.frontier.committedSeq, 4);
});

test("dual-run: a committed rewrite control skips mapping and requests a refetch", () => {
  const threadId = "thread::dual-run-rewrite";
  const refetched = [];
  const mirror = new GatewayMirror({
    getState: async () => ({}),
    listCustomAgents: async () => [],
    listTeams: async () => [],
    listWorkflowDefinitions: async () => [],
    getThreadHistory: async () => {
      throw new Error("unused");
    },
    requestAuthoritativeRefetch: (id) => refetched.push(id),
  });

  const fullTranscript = {
    threadId,
    remoteFound: true,
    messages: [wireMessage(1, "user", "hello")],
    pendingInputs: [],
    threadInfo: null,
    pageInfo: fullPageInfo({ totalMessages: 1, committedMessages: 1, returnedMessages: 1, endIndex: 1 }),
  };
  let legacy = legacyRemoteApply(emptyLegacyState, fullTranscript);
  mirror.applyRemoteTranscript(threadId, fullTranscript);

  const rewrite = committedEvent(threadId, 2, {
    id: "seq:2",
    role: "system",
    kind: "control",
    text: "",
    content: { control: { kind: "range_rewrite" } },
  });
  assert.equal(transcriptRewriteAction(rewrite.message), "refetch_authoritative");

  legacy = legacyCommittedApply(legacy, rewrite);
  assert.equal(legacy.refetchRequested, true);
  mirror.ingest(rewrite);

  assertThreadMatchesLegacy(mirror, threadId, legacy);
  assert.deepEqual(refetched, [threadId], "refetch requested once");
  // The verbatim ledger still records the control event.
  const snapshot = mirror.getThreadSnapshot(threadId);
  assert.equal(snapshot.records.length, 1);
  assert.equal(snapshot.frontier.committedSeq, 2);

  // Redelivery of the same seq is idempotent: no second refetch request.
  mirror.ingest(rewrite);
  assert.deepEqual(refetched, [threadId]);
});

test("dual-run: loadOlderThreadHistoryPage matches the legacy older-page apply", async () => {
  const threadId = "thread::dual-run-older";
  const olderPage = {
    threadId,
    remoteFound: true,
    messages: [wireMessage(1, "user", "old question"), wireMessage(2, "assistant", "old answer")],
    pendingInputs: [],
    threadInfo: null,
    pageInfo: fullPageInfo({
      totalMessages: 4,
      committedMessages: 4,
      startIndex: 1,
      endIndex: 2,
      hasMoreBefore: false,
      nextBeforeIndex: null,
    }),
  };
  const historyCalls = [];
  const mirror = new GatewayMirror({
    getState: async () => ({}),
    listCustomAgents: async () => [],
    listTeams: async () => [],
    listWorkflowDefinitions: async () => [],
    getThreadHistory: async (input) => {
      historyCalls.push(input);
      return olderPage;
    },
  });

  const fullTranscript = {
    threadId,
    remoteFound: true,
    messages: [wireMessage(3, "user", "recent"), wireMessage(4, "assistant", "reply")],
    pendingInputs: [],
    threadInfo: null,
    pageInfo: fullPageInfo({
      totalMessages: 4,
      committedMessages: 4,
      startIndex: 3,
      endIndex: 4,
      hasMoreBefore: true,
      nextBeforeIndex: 2,
    }),
  };
  let legacy = legacyRemoteApply(emptyLegacyState, fullTranscript);
  mirror.applyRemoteTranscript(threadId, fullTranscript);
  assertThreadMatchesLegacy(mirror, threadId, legacy);
  assert.equal(legacy.pagination.hasMoreBefore, true);

  // Loading flag lifecycle: observable while the fetch is in flight.
  let sawLoading = false;
  let fetchedDuring = null;
  mirror.subscribeThread(threadId, () => {
    if (mirror.getThreadSnapshot(threadId).historyPagination?.loadingBefore) {
      sawLoading = true;
    }
  });

  await mirror.loadOlderThreadHistoryPage(threadId, {
    onPageFetched: () => {
      // The UI scroll-anchor seam runs between fetch and apply: the message
      // cache must not have been prepended yet.
      fetchedDuring = mirror.getThreadSnapshot(threadId).messages.length;
    },
  });

  assert.deepEqual(historyCalls, [
    {
      threadId,
      beforeIndex: 2,
      limit: THREAD_HISTORY_PAGE_SIZE,
      userQueryLimit: THREAD_HISTORY_USER_QUERY_LIMIT,
    },
  ]);
  assert.equal(sawLoading, true, "loadingBefore visible during fetch");
  assert.equal(fetchedDuring, legacy.messages.length, "apply happens after the anchor seam");

  // Legacy older-page apply: pagination replaced from the page, materialized
  // older entries prepended (verbatim applyOlderRemoteTranscriptPage).
  const legacyPagination = paginationStateFromTranscript(olderPage);
  const visibleOlder = visibleTranscriptMessages(olderPage.messages);
  const existingIds = new Set(legacy.messages.map((entry) => entry.id));
  const olderEntries = materializeRemoteTranscript(visibleOlder, []).filter(
    (entry) => !existingIds.has(entry.id),
  );
  legacy = {
    ...legacy,
    pagination: { ...legacyPagination, loadingBefore: false },
    messages: [...olderEntries, ...legacy.messages],
  };

  const snapshot = mirror.getThreadSnapshot(threadId);
  assert.deepEqual(snapshot.messages, legacy.messages);
  assert.deepEqual(snapshot.historyPagination, legacy.pagination);
  assert.ok(olderEntries.length > 0, "older page must actually prepend rows");
});

test("applyOlderHistoryPage (dual-write entry) matches the fetch-owning path", async () => {
  const threadId = "thread::older-dual-write";
  const olderPage = {
    threadId,
    remoteFound: true,
    messages: [wireMessage(1, "user", "old question"), wireMessage(2, "assistant", "old answer")],
    pendingInputs: [],
    threadInfo: null,
    pageInfo: fullPageInfo({
      totalMessages: 4,
      committedMessages: 4,
      startIndex: 1,
      endIndex: 2,
      hasMoreBefore: false,
      nextBeforeIndex: null,
    }),
  };
  const fullTranscript = {
    threadId,
    remoteFound: true,
    messages: [wireMessage(3, "user", "recent"), wireMessage(4, "assistant", "reply")],
    pendingInputs: [],
    threadInfo: null,
    pageInfo: fullPageInfo({
      totalMessages: 4,
      committedMessages: 4,
      startIndex: 3,
      endIndex: 4,
      hasMoreBefore: true,
      nextBeforeIndex: 2,
    }),
  };

  // Path A (batch-2b dual-write): the legacy hook fetched the page itself
  // and feeds only the apply step.
  const dualWriteMirror = new GatewayMirror();
  dualWriteMirror.applyRemoteTranscript(threadId, fullTranscript);
  dualWriteMirror.applyOlderHistoryPage(threadId, olderPage);
  const dualWriteSnapshot = dualWriteMirror.getThreadSnapshot(threadId);

  // Path B (mirror-owned): loadOlderThreadHistoryPage fetches through the
  // injected services and applies the same page.
  const fetchMirror = new GatewayMirror({
    getState: async () => ({}),
    listCustomAgents: async () => [],
    listTeams: async () => [],
    listWorkflowDefinitions: async () => [],
    getThreadHistory: async () => olderPage,
  });
  fetchMirror.applyRemoteTranscript(threadId, fullTranscript);
  await fetchMirror.loadOlderThreadHistoryPage(threadId);
  const fetchSnapshot = fetchMirror.getThreadSnapshot(threadId);

  assert.deepEqual(dualWriteSnapshot.messages, fetchSnapshot.messages);
  assert.deepEqual(
    dualWriteSnapshot.historyPagination,
    fetchSnapshot.historyPagination,
  );
});

test("loadOlderThreadHistoryPage guards: no pagination, in-flight, and fetch errors", async () => {
  const threadId = "thread::older-guards";
  let resolveFetch;
  let calls = 0;
  const mirror = new GatewayMirror({
    getState: async () => ({}),
    listCustomAgents: async () => [],
    listTeams: async () => [],
    listWorkflowDefinitions: async () => [],
    getThreadHistory: () => {
      calls += 1;
      return new Promise((resolve) => {
        resolveFetch = resolve;
      });
    },
  });

  // No pagination state at all: fetch must not run.
  await mirror.loadOlderThreadHistoryPage(threadId);
  assert.equal(calls, 0);

  const fullTranscript = {
    threadId,
    remoteFound: true,
    messages: [wireMessage(5, "user", "latest")],
    pendingInputs: [],
    threadInfo: null,
    pageInfo: fullPageInfo({
      totalMessages: 5,
      committedMessages: 5,
      startIndex: 5,
      endIndex: 5,
      hasMoreBefore: true,
      nextBeforeIndex: 4,
    }),
  };
  mirror.applyRemoteTranscript(threadId, fullTranscript);

  // In-flight guard: a second call while loadingBefore is set is a no-op.
  const firstLoad = mirror.loadOlderThreadHistoryPage(threadId);
  await mirror.loadOlderThreadHistoryPage(threadId);
  assert.equal(calls, 1, "concurrent older-page loads must not double-fetch");
  resolveFetch({
    threadId,
    remoteFound: true,
    messages: [],
    pendingInputs: [],
    threadInfo: null,
    pageInfo: fullPageInfo({
      totalMessages: 5,
      committedMessages: 5,
      startIndex: 1,
      endIndex: 4,
      hasMoreBefore: false,
      nextBeforeIndex: null,
    }),
  });
  await firstLoad;
  assert.equal(
    mirror.getThreadSnapshot(threadId).historyPagination.loadingBefore,
    false,
  );
  assert.equal(
    mirror.getThreadSnapshot(threadId).historyPagination.hasMoreBefore,
    false,
    "empty page still replaces pagination (legacy behavior)",
  );

  // Fetch error: loadingBefore resets and the error propagates.
  const errorMirror = new GatewayMirror({
    getState: async () => ({}),
    listCustomAgents: async () => [],
    listTeams: async () => [],
    listWorkflowDefinitions: async () => [],
    getThreadHistory: async () => {
      throw new Error("history endpoint down");
    },
  });
  errorMirror.applyRemoteTranscript(threadId, fullTranscript);
  await assert.rejects(
    () => errorMirror.loadOlderThreadHistoryPage(threadId),
    /history endpoint down/,
  );
  assert.equal(
    errorMirror.getThreadSnapshot(threadId).historyPagination.loadingBefore,
    false,
    "fetch error must clear the loading flag",
  );
});

test("mergeRemoteTranscriptWithLocal preserves local intent entries via the injected lookup", () => {
  const intent = {
    intentId: "intent-1",
    threadId: "thread::merge-intents",
    state: "awaiting_response",
    dispatchMode: "sync_send",
    responseText: "",
  };
  const intents = { [intent.intentId]: intent };
  const intentForId = (id) => intents[id] || null;

  const localUser = {
    id: "local-user-1",
    role: "user",
    text: "optimistic send",
    localState: "pending",
    intentId: intent.intentId,
  };
  const remoteOnly = [wireMessage(1, "user", "earlier"), wireMessage(2, "assistant", "earlier reply")];

  // Remote does not echo the local user message yet: it must be preserved.
  const preserved = mergeRemoteTranscriptWithLocal(remoteOnly, [localUser], {
    intentForId,
  });
  assert.ok(
    preserved.some((entry) => entry.id === localUser.id),
    "unechoed optimistic user entry must survive the merge",
  );

  // Remote now carries the origin-id echo: the local copy must drop.
  const echoed = [
    ...remoteOnly,
    {
      id: userMessageIdForOrigin(intent.intentId),
      role: "user",
      text: "optimistic send",
      timestamp: "2026-06-19T12:23:00Z",
    },
  ];
  const deduped = mergeRemoteTranscriptWithLocal(echoed, [localUser], {
    intentForId,
  });
  assert.ok(
    !deduped.some((entry) => entry.id === localUser.id),
    "echoed optimistic user entry must be replaced by the remote copy",
  );

  // Null lookup (mirror without a live machine): local entry with an intent
  // id degrades to the no-intent rule and drops unless error/interrupted.
  const nullLookup = mergeRemoteTranscriptWithLocal(remoteOnly, [localUser], {
    intentForId: () => null,
  });
  assert.ok(!nullLookup.some((entry) => entry.id === localUser.id));
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

test("live-stream updates commit the thread snapshot and rebuild the aggregate map per update (3c-1)", () => {
  const mirror = new GatewayMirror();
  const threadId = "thread::live-stream-a";

  const initialMap = mirror.getLiveStreamMap();
  assert.equal(mirror.getLiveStreamMap(), initialMap, "getter must not allocate");
  assert.equal(mirror.getThreadLiveStream(threadId), null);
  assert.equal(
    mirror.getThreadSnapshot(threadId).liveStream,
    null,
    "empty thread snapshot exposes a null liveStream",
  );

  let liveStreamNotifications = 0;
  mirror.subscribeLiveStreams(() => {
    liveStreamNotifications += 1;
  });

  const beforeSnapshot = mirror.getThreadSnapshot(threadId);
  const created = mirror.updateThreadLiveStream(threadId, (current) => {
    assert.equal(current, null, "first updater sees null");
    return {
      threadId,
      activeIntentId: "intent-1",
      assistantEntryId: null,
      pendingAckIntentIds: [],
      streamStatus: "connecting",
    };
  });
  assert.equal(created?.streamStatus, "connecting");
  assert.equal(liveStreamNotifications, 1);
  const mapAfterCreate = mirror.getLiveStreamMap();
  assert.notEqual(mapAfterCreate, initialMap, "update rebuilds the map");
  assert.equal(mapAfterCreate[threadId], created);
  const afterSnapshot = mirror.getThreadSnapshot(threadId);
  assert.notEqual(afterSnapshot, beforeSnapshot, "thread snapshot commits");
  assert.equal(afterSnapshot.liveStream, created);
  assert.equal(afterSnapshot.version, beforeSnapshot.version + 1);

  // Legacy setState cadence: even a no-change update rebuilds the map and
  // notifies (the legacy updater always allocated a fresh Record).
  const unchanged = mirror.updateThreadLiveStream(threadId, (current) => current);
  assert.equal(unchanged, created);
  assert.equal(liveStreamNotifications, 2);
  assert.notEqual(mirror.getLiveStreamMap(), mapAfterCreate);

  const cleared = mirror.updateThreadLiveStream(threadId, () => null);
  assert.equal(cleared, null);
  assert.equal(mirror.getThreadLiveStream(threadId), null);
  assert.ok(!(threadId in mirror.getLiveStreamMap()), "null result deletes the entry");
  assert.equal(mirror.getThreadSnapshot(threadId).liveStream, null);
});

test("live-stream updates notify only the touched thread's subscribers plus the aggregate domain", () => {
  const mirror = new GatewayMirror();
  const threadA = "thread::live-stream-iso-a";
  const threadB = "thread::live-stream-iso-b";

  let notifiedA = 0;
  let notifiedB = 0;
  let aggregate = 0;
  mirror.subscribeThread(threadA, () => {
    notifiedA += 1;
  });
  mirror.subscribeThread(threadB, () => {
    notifiedB += 1;
  });
  mirror.subscribeLiveStreams(() => {
    aggregate += 1;
  });

  mirror.updateThreadLiveStream(threadA, () => ({
    threadId: threadA,
    pendingAckIntentIds: [],
    streamStatus: "streaming",
  }));

  assert.equal(notifiedA, 1, "touched thread notifies");
  assert.equal(notifiedB, 0, "other threads stay silent");
  assert.equal(aggregate, 1, "aggregate domain notifies once");
});

test("replaceLiveStreamThreadId moves the draft entry in one aggregate notification (3c-1)", () => {
  const mirror = new GatewayMirror();
  const draftId = "__garyx_new_thread_draft__";
  const realId = "thread::promoted";

  let aggregate = 0;
  mirror.subscribeLiveStreams(() => {
    aggregate += 1;
  });

  // No-op when the source has no entry (legacy guard).
  const mapBefore = mirror.getLiveStreamMap();
  mirror.replaceLiveStreamThreadId(draftId, realId);
  assert.equal(aggregate, 0);
  assert.equal(mirror.getLiveStreamMap(), mapBefore, "no-op keeps the map identity");

  mirror.updateThreadLiveStream(draftId, () => ({
    threadId: draftId,
    activeIntentId: "intent-9",
    assistantEntryId: null,
    pendingAckIntentIds: [],
    streamStatus: "connecting",
  }));
  assert.equal(aggregate, 1);

  const draftSnapshotBefore = mirror.getThreadSnapshot(draftId);
  const realSnapshotBefore = mirror.getThreadSnapshot(realId);
  mirror.replaceLiveStreamThreadId(draftId, realId);

  assert.equal(aggregate, 2, "rename notifies the aggregate domain once");
  const map = mirror.getLiveStreamMap();
  assert.ok(!(draftId in map), "draft key is removed");
  assert.equal(map[realId].threadId, realId, "threadId field is rewritten");
  assert.equal(map[realId].activeIntentId, "intent-9");
  assert.equal(mirror.getThreadLiveStream(draftId), null);
  assert.equal(mirror.getThreadLiveStream(realId), map[realId]);
  assert.notEqual(
    mirror.getThreadSnapshot(draftId),
    draftSnapshotBefore,
    "source thread snapshot commits",
  );
  assert.notEqual(
    mirror.getThreadSnapshot(realId),
    realSnapshotBefore,
    "target thread snapshot commits",
  );
  assert.equal(mirror.getThreadSnapshot(realId).liveStream, map[realId]);
});
