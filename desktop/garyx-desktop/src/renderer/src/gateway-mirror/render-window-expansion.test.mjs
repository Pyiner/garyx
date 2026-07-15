import assert from "node:assert/strict";
import { test } from "node:test";

import { buildThreadViewRows } from "../render-view-model.ts";
import { GatewayMirror } from "./mirror.ts";
import { ThreadTranscriptCache } from "./transcript-cache.ts";
import { SELECTED_THREAD_STREAM_CONSUMER_ID } from "./transcript-lifecycle.ts";

const THREAD_ID = "thread::render-window-expansion";

function message(seq, role, text) {
  return {
    id: `seq:${seq - 1}`,
    seq,
    role,
    text,
    timestamp: `2026-07-15T09:00:${String(seq).padStart(2, "0")}Z`,
  };
}

function pageInfo(overrides = {}) {
  return {
    totalMessages: 4,
    committedMessages: 4,
    returnedMessages: 2,
    startIndex: 2,
    endIndex: 4,
    hasMoreBefore: true,
    nextBeforeIndex: 2,
    hasMoreAfter: false,
    nextAfterIndex: null,
    reset: false,
    limit: 100,
    userQueryLimit: 10,
    ...overrides,
  };
}

function transcript(messages, info = {}) {
  return {
    threadId: THREAD_ID,
    remoteFound: true,
    messages,
    pendingInputs: [],
    threadInfo: null,
    pageInfo: pageInfo(info),
  };
}

function messageRef(seq, role) {
  return { id: `seq:${seq - 1}`, seq, role };
}

function turnRow(userSeq, assistantSeq) {
  return {
    kind: "user_turn",
    id: `turn:${userSeq}`,
    user: messageRef(userSeq, "user"),
    activity: [
      {
        kind: "assistant_reply",
        id: `reply:${assistantSeq}`,
        message: messageRef(assistantSeq, "assistant"),
        streaming: false,
      },
    ],
    started_at: null,
    finished_at: null,
    capsule_cards: [],
  };
}

function renderState(floorSeq, rows, basedOnSeq = 4) {
  return {
    based_on_seq: basedOnSeq,
    rows,
    tailActivity: "none",
    activeToolGroupId: null,
    progress_locus: "none",
    filtered_placeholders: [],
    ...(floorSeq > 0
      ? { window: { floor_seq: floorSeq, has_more_above: true } }
      : {}),
  };
}

function messagesBySeq(mirror) {
  return new Map(
    mirror
      .getThreadSnapshot(THREAD_ID)
      .messages.filter((entry) => Number.isFinite(entry.seq))
      .map((entry) => [entry.seq, entry]),
  );
}

function attachLifecycleDeps(mirror, overrides = {}) {
  mirror.setTranscriptLifecycleDeps({
    setDesktopState: () => {},
    syncThreadTitleDraft: () => {},
    requestSelectedThreadMessagesBottomSnap: () => {},
    selectedThreadIdRef: { current: THREAD_ID },
    setError: (error) => {
      if (error) throw new Error(error);
    },
    setHistoryLoading: () => {},
    setPendingAutomationRun: () => {},
    recordGatewayStatusObservation: () => {},
    scheduleDesktopStateRefresh: () => {},
    scheduleHistoryRefresh: () => {},
    connection: null,
    settingsDraft: { gatewayUrl: "http://gateway.test" },
    desktopState: null,
    refreshDesktopState: async () => ({ threads: [], sessions: [] }),
    selectedThreadGenerationRef: { current: 1 },
    lastRenderedMessageThreadRef: { current: THREAD_ID },
    messagesRef: { current: null },
    pendingMessagesPrependAnchorRef: { current: null },
    sideChatThreadIdRef: { current: null },
    sideChatStreamConsumerId: (threadId) => `side-chat:${threadId}`,
    ...overrides,
  });
}

function createWindowHarness({
  olderPages = [],
  authoritativeTranscript,
  cachedEntry = null,
} = {}) {
  const starts = [];
  const stops = [];
  const errors = [];
  const fullFetches = [];
  let olderPageIndex = 0;
  const mirror = new GatewayMirror({
    getState: async () => ({}),
    listCustomAgents: async () => [],
    getThreadHistory: async () =>
      olderPages[Math.min(olderPageIndex++, olderPages.length - 1)],
    getThreadHistoryFull: async (threadId) => {
      fullFetches.push(threadId);
      return (
        authoritativeTranscript ??
        transcript([], {
          returnedMessages: 0,
          startIndex: 0,
          endIndex: 0,
          hasMoreBefore: false,
          nextBeforeIndex: null,
        })
      );
    },
    startThreadStream: async (input) => starts.push(input),
    stopThreadStream: async (input) => stops.push(input),
    loadThreadTranscriptCache: async () => cachedEntry,
    clearThreadTranscriptCache: async () => {},
  });
  attachLifecycleDeps(mirror, { setError: (error) => errors.push(error) });
  return { mirror, starts, stops, errors, fullFetches };
}

const flushAsync = () => new Promise((resolve) => setImmediate(resolve));

async function establishWindow(harness, initialTranscript, floorSeq, basedOnSeq) {
  harness.mirror.applyRemoteTranscript(THREAD_ID, initialTranscript);
  harness.mirror.ingest({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    events: [],
    renderState: renderState(floorSeq, [], basedOnSeq),
  });
  await harness.mirror.startCommittedThreadStream(
    THREAD_ID,
    initialTranscript,
    SELECTED_THREAD_STREAM_CONSUMER_ID,
  );
  return harness.starts.at(-1);
}

test("earliestLoadedCommittedBodySeq reads uiMessages and excludes non-record locals", () => {
  const cache = new ThreadTranscriptCache();
  cache.setUiMessages([
    { id: "local", role: "user", text: "optimistic", localState: "optimistic" },
    { id: "invalid", seq: Number.NaN, role: "assistant", text: "invalid" },
    { id: "remote-5", seq: 5, role: "assistant", text: "five", localState: "remote_final" },
    { id: "remote-2", seq: 2, role: "user", text: "two", localState: "remote_final" },
  ]);
  assert.equal(cache.earliestLoadedCommittedBodySeq(), 2);

  cache.setUiMessages([
    { id: "local-only", role: "user", text: "draft", localState: "optimistic" },
  ]);
  assert.equal(cache.earliestLoadedCommittedBodySeq(), null);
});

test("mirror detects full render-state value changes while ignoring rows_hash", () => {
  const mirror = new GatewayMirror();
  const initial = renderState(3, [turnRow(3, 4)]);
  mirror.ingest({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    events: [],
    renderState: initial,
  });
  const first = mirror.getThreadSnapshot(THREAD_ID);

  mirror.ingest({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    events: [],
    renderState: { ...initial, rows: [turnRow(1, 2)] },
  });
  const rowOverwrite = mirror.getThreadSnapshot(THREAD_ID);
  assert.notEqual(rowOverwrite, first, "same-seq row overwrite applies");

  mirror.ingest({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    events: [],
    renderState: {
      ...rowOverwrite.renderState,
      tailActivity: "thinking",
    },
  });
  const scalarOverwrite = mirror.getThreadSnapshot(THREAD_ID);
  assert.notEqual(
    scalarOverwrite,
    rowOverwrite,
    "same-seq scalar-only overwrite applies",
  );

  mirror.ingest({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    events: [],
    renderState: { ...scalarOverwrite.renderState },
  });
  assert.equal(
    mirror.getThreadSnapshot(THREAD_ID),
    scalarOverwrite,
    "identical snapshot preserves reference stability",
  );
  mirror.ingest({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    events: [],
    renderState: { ...scalarOverwrite.renderState, rows_hash: "transport-token" },
  });
  assert.equal(
    mirror.getThreadSnapshot(THREAD_ID),
    scalarOverwrite,
    "rows_hash presence alone is not a render value change",
  );

  mirror.ingest({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    events: [],
    renderState: {
      ...scalarOverwrite.renderState,
      rows: [turnRow(1, 2), turnRow(3, 4)],
      window: { floor_seq: 1, has_more_above: true },
    },
  });
  assert.notEqual(
    mirror.getThreadSnapshot(THREAD_ID),
    scalarOverwrite,
    "same-seq wider window applies",
  );
});

test("committed ledger identity is payload-aware and body overwrites remap", () => {
  const mirror = new GatewayMirror();
  const bodyA = message(10, "assistant", "payload A");
  mirror.applyRemoteTranscript(
    THREAD_ID,
    transcript([bodyA], {
      totalMessages: 10,
      committedMessages: 10,
      returnedMessages: 1,
      startIndex: 9,
      endIndex: 10,
      hasMoreBefore: false,
      nextBeforeIndex: null,
    }),
  );
  const eventA = {
    type: "committed_message",
    requestId: "logical-a",
    threadId: THREAD_ID,
    runId: "run-ledger",
    seq: 10,
    message: bodyA,
  };
  mirror.ingest(eventA);
  const inserted = mirror.getThreadSnapshot(THREAD_ID);

  mirror.ingest({ ...eventA, requestId: "logical-b" });
  assert.equal(
    mirror.getThreadSnapshot(THREAD_ID),
    inserted,
    "request correlation is not part of committed payload identity",
  );

  const bodyB = { ...bodyA, text: "payload B" };
  mirror.ingest({ ...eventA, requestId: "logical-old", message: bodyB });
  const overwritten = mirror.getThreadSnapshot(THREAD_ID);
  assert.notEqual(overwritten, inserted);
  assert.equal(
    overwritten.messages.find((entry) => entry.seq === 10)?.text,
    "payload B",
    "same-seq distinct body is re-materialized",
  );
  assert.equal(overwritten.records.length, 1);
  assert.equal(overwritten.records[0].message.text, "payload B");

  mirror.ingest({ ...eventA, requestId: "logical-current", message: bodyB });
  assert.equal(
    mirror.getThreadSnapshot(THREAD_ID),
    overwritten,
    "same seq plus structurally equal payload is a silent duplicate",
  );
});

test("anchor: pagination expands a windowed snapshot and lights up older turns", async () => {
  const starts = [];
  const recent = transcript([
    message(3, "user", "recent question"),
    message(4, "assistant", "recent answer"),
  ]);
  const older = transcript(
    [message(1, "user", "older question"), message(2, "assistant", "older answer")],
    {
      returnedMessages: 2,
      startIndex: 0,
      endIndex: 2,
      hasMoreBefore: false,
      nextBeforeIndex: null,
    },
  );
  const mirror = new GatewayMirror({
    getState: async () => ({}),
    listCustomAgents: async () => [],
    getThreadHistory: async () => older,
    getThreadHistoryFull: async () => recent,
    startThreadStream: async (input) => starts.push(input),
    stopThreadStream: async () => {},
    loadThreadTranscriptCache: async () => null,
    clearThreadTranscriptCache: async () => {},
  });
  attachLifecycleDeps(mirror);

  mirror.applyRemoteTranscript(THREAD_ID, recent);
  mirror.ingest({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    events: [],
    renderState: renderState(3, [turnRow(3, 4)]),
  });
  await mirror.startCommittedThreadStream(
    THREAD_ID,
    recent,
    SELECTED_THREAD_STREAM_CONSUMER_ID,
  );
  starts.length = 0;

  await mirror.loadOlderThreadHistoryPage(THREAD_ID);

  assert.equal(
    starts.length,
    1,
    "an older-page apply below floor 3 must issue exactly one expansion start",
  );
  assert.ok((starts[0].renderFloor ?? 0) <= 1);

  mirror.notifyStreamEvent({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    requestId: starts[0].requestId,
    events: [],
    renderState: renderState(0, [turnRow(1, 2), turnRow(3, 4)]),
  });

  const rows = buildThreadViewRows(
    mirror.getThreadSnapshot(THREAD_ID).renderState,
    messagesBySeq(mirror),
  );
  assert.deepEqual(
    rows.map(
      (row) =>
        row.kind === "user_turn" && row.userBlock.entry.message.seq,
    ),
    [1, 3],
    "the same-cursor wide snapshot must make the seq 1-2 turn visible",
  );
  assert.equal(
    rows[0]?.kind === "user_turn" &&
      rows[0].activityRows[0]?.kind === "flat" &&
      rows[0].activityRows[0].block.entry.message.seq,
    2,
    "the newly visible first turn must retain its seq 2 assistant reply",
  );
});

test("stale logical frames apply ledger events but drop render, marker, and settle semantics", async () => {
  const recent = transcript([
    message(3, "user", "recent question"),
    message(4, "assistant", "recent answer"),
  ]);
  const older = transcript(
    [message(1, "user", "older question"), message(2, "assistant", "older answer")],
    {
      startIndex: 0,
      endIndex: 2,
      hasMoreBefore: false,
      nextBeforeIndex: null,
    },
  );
  const h = createWindowHarness({ olderPages: [older] });
  const oldRequest = await establishWindow(h, recent, 3, 4);
  await h.mirror.loadOlderThreadHistoryPage(THREAD_ID);
  const expansionRequest = h.starts.at(-1);
  assert.notEqual(expansionRequest.requestId, oldRequest.requestId);

  h.mirror.notifyStreamEvent({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    requestId: expansionRequest.requestId,
    events: [],
    renderState: renderState(0, [turnRow(1, 2), turnRow(3, 4)], 4),
  });
  const wideRender = h.mirror.getThreadSnapshot(THREAD_ID).renderState;

  h.mirror.notifyStreamEvent({
    type: "committed_message",
    requestId: expansionRequest.requestId,
    threadId: THREAD_ID,
    runId: "run-stale-split",
    seq: 1,
    message: message(1, "user", "older question"),
  });
  const staleFrame = {
    type: "thread_render_frame",
    threadId: THREAD_ID,
    requestId: oldRequest.requestId,
    replay: "windowed",
    events: [
      {
        type: "committed_message",
        threadId: THREAD_ID,
        runId: "run-stale-split",
        seq: 5,
        message: message(5, "assistant", "unique stale-frame body"),
      },
    ],
    renderState: renderState(3, [turnRow(3, 4)], 5),
  };
  h.mirror.notifyStreamEvent(staleFrame);

  const after = h.mirror.getThreadSnapshot(THREAD_ID);
  assert.equal(after.renderState, wideRender, "stale render must not re-narrow");
  assert.deepEqual(
    after.records.map((record) => record.seq),
    [1, 5],
    "stale window marker must not drop the cached below-floor record",
  );
  assert.equal(after.frontier.committedSeq, 5);
  assert.equal(
    after.messages.filter((entry) => entry.seq === 5).length,
    1,
    "the stale frame's unique ledger body applies exactly once",
  );

  h.mirror.notifyStreamEvent({
    type: "error",
    threadId: THREAD_ID,
    requestId: oldRequest.requestId,
    runId: "stale-error",
    error: "must stay connection-scoped",
    terminal: true,
  });
  assert.deepEqual(h.errors, [], "stale request errors are discarded");

  h.mirror.notifyStreamEvent(staleFrame);
  assert.equal(
    h.mirror.getThreadSnapshot(THREAD_ID),
    after,
    "identical stale frame redelivery is silent",
  );
});

test("stale same-seq rewrite refetches once, identical replay is silent, and body overwrite remaps", async () => {
  const recent = transcript([
    message(3, "user", "recent question"),
    message(4, "assistant", "recent answer"),
  ]);
  const older = transcript(
    [message(1, "user", "older question"), message(2, "assistant", "older answer")],
    {
      startIndex: 0,
      endIndex: 2,
      hasMoreBefore: false,
      nextBeforeIndex: null,
    },
  );
  const authoritative = transcript(
    [message(10, "assistant", "authoritative body")],
    {
      totalMessages: 10,
      committedMessages: 10,
      returnedMessages: 1,
      startIndex: 9,
      endIndex: 10,
      hasMoreBefore: false,
      nextBeforeIndex: null,
    },
  );
  const h = createWindowHarness({
    olderPages: [older],
    authoritativeTranscript: authoritative,
  });
  const oldRequest = await establishWindow(h, recent, 3, 4);
  await h.mirror.loadOlderThreadHistoryPage(THREAD_ID);
  const expansionRequest = h.starts.at(-1);
  h.mirror.notifyStreamEvent({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    requestId: expansionRequest.requestId,
    events: [],
    renderState: renderState(0, [turnRow(1, 2), turnRow(3, 4)], 4),
  });
  const wideRender = h.mirror.getThreadSnapshot(THREAD_ID).renderState;

  const bodyA = message(10, "assistant", "payload A");
  h.mirror.notifyStreamEvent({
    type: "committed_message",
    requestId: expansionRequest.requestId,
    threadId: THREAD_ID,
    runId: "run-payload-matrix",
    seq: 10,
    message: bodyA,
  });
  const rewrite = {
    type: "thread_render_frame",
    threadId: THREAD_ID,
    requestId: oldRequest.requestId,
    replay: "windowed",
    events: [
      {
        type: "committed_message",
        threadId: THREAD_ID,
        runId: "run-payload-matrix",
        seq: 10,
        message: {
          id: "seq:9",
          seq: 10,
          role: "system",
          text: "",
          kind: "control",
          content: {
            control: { kind: "range_rewrite", from_seq: 10, to_seq: 10 },
          },
        },
      },
    ],
    renderState: renderState(3, [turnRow(3, 4)], 10),
  };
  h.mirror.notifyStreamEvent(rewrite);
  await flushAsync();
  await flushAsync();
  assert.deepEqual(
    h.fullFetches,
    [THREAD_ID],
    "one distinct rewrite payload triggers one authoritative refetch",
  );
  assert.equal(h.mirror.getThreadSnapshot(THREAD_ID).renderState, wideRender);

  h.mirror.notifyStreamEvent(rewrite);
  await flushAsync();
  assert.deepEqual(
    h.fullFetches,
    [THREAD_ID],
    "identical same-seq rewrite redelivery must not refetch again",
  );

  h.mirror.notifyStreamEvent({
    ...rewrite,
    events: [
      {
        type: "committed_message",
        threadId: THREAD_ID,
        runId: "run-payload-matrix",
        seq: 10,
        message: { ...bodyA, text: "payload C" },
      },
    ],
  });
  const afterBodyOverwrite = h.mirror.getThreadSnapshot(THREAD_ID);
  assert.equal(
    afterBodyOverwrite.messages.find((entry) => entry.seq === 10)?.text,
    "payload C",
  );
  assert.equal(afterBodyOverwrite.renderState, wideRender);
  assert.equal(h.fullFetches.length, 1);
});

test("failed expansion gets one settle retry, then holds until the next demand", async () => {
  const recent = transcript(
    [message(300, "user", "recent"), message(301, "assistant", "reply")],
    {
      totalMessages: 301,
      committedMessages: 301,
      startIndex: 299,
      endIndex: 301,
      hasMoreBefore: true,
      nextBeforeIndex: 299,
    },
  );
  const older = transcript([message(200, "user", "older")], {
    totalMessages: 301,
    committedMessages: 301,
    returnedMessages: 1,
    startIndex: 199,
    endIndex: 200,
    hasMoreBefore: true,
    nextBeforeIndex: 199,
  });
  const h = createWindowHarness({ olderPages: [older, older] });
  await establishWindow(h, recent, 300, 301);
  await h.mirror.loadOlderThreadHistoryPage(THREAD_ID);
  assert.equal(h.starts.length, 2);
  const firstAttempt = h.starts[1];
  assert.equal(firstAttempt.renderFloor, 100);

  h.mirror.notifyStreamEvent({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    requestId: firstAttempt.requestId,
    replay: "windowed",
    events: [],
    renderState: renderState(250, [], 301),
  });
  assert.equal(h.starts.length, 3, "first degrade spends the one retry");
  const retry = h.starts[2];
  assert.equal(
    retry.renderFloor,
    100,
    "failed settle must not grow prepayMargin",
  );

  h.mirror.notifyStreamEvent({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    requestId: retry.requestId,
    replay: "windowed",
    events: [],
    renderState: renderState(240, [], 301),
  });
  await flushAsync();
  assert.equal(
    h.starts.length,
    3,
    "second degrade holds; settle/quiescence must not start again",
  );

  await h.mirror.loadOlderThreadHistoryPage(THREAD_ID);
  assert.equal(h.starts.length, 4, "next page demand restores a fresh attempt");
  assert.notEqual(h.starts[3].requestId, retry.requestId);
});

test("consumer join rebinds a pending target and consumes the new epoch's first attempt", async () => {
  const recent = transcript(
    [message(300, "user", "recent"), message(301, "assistant", "reply")],
    {
      totalMessages: 301,
      committedMessages: 301,
      startIndex: 299,
      endIndex: 301,
      hasMoreBefore: true,
      nextBeforeIndex: 299,
    },
  );
  const older = transcript([message(200, "user", "older")], {
    totalMessages: 301,
    committedMessages: 301,
    returnedMessages: 1,
    startIndex: 199,
    endIndex: 200,
    hasMoreBefore: false,
    nextBeforeIndex: null,
  });
  const h = createWindowHarness({ olderPages: [older] });
  await establishWindow(h, recent, 300, 301);
  await h.mirror.loadOlderThreadHistoryPage(THREAD_ID);
  const pending = h.starts.at(-1);
  assert.equal(pending.renderFloor, 100);

  await h.mirror.startCommittedThreadStream(
    THREAD_ID,
    recent,
    `side-chat:${THREAD_ID}`,
  );
  assert.equal(h.starts.length, 3, "join performs exactly one physical start");
  const rebound = h.starts[2];
  assert.equal(rebound.renderFloor, pending.renderFloor);
  assert.notEqual(rebound.requestId, pending.requestId);

  h.mirror.notifyStreamEvent({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    requestId: rebound.requestId,
    replay: "windowed",
    events: [],
    renderState: renderState(250, [], 301),
  });
  assert.equal(h.starts.length, 4, "rebound degrade has exactly one retry");
  const retry = h.starts[3];
  h.mirror.notifyStreamEvent({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    requestId: retry.requestId,
    replay: "windowed",
    events: [],
    renderState: renderState(240, [], 301),
  });
  await flushAsync();
  assert.equal(
    h.starts.length,
    4,
    "pending join → degrade → retry → degrade is held, never double-budgeted",
  );
});

test("a stale request frame cannot settle or replace the pending expansion", async () => {
  const recent = transcript(
    [message(300, "user", "recent"), message(301, "assistant", "reply")],
    {
      totalMessages: 301,
      committedMessages: 301,
      startIndex: 299,
      endIndex: 301,
      hasMoreBefore: true,
      nextBeforeIndex: 299,
    },
  );
  const older = transcript([message(200, "user", "older")], {
    totalMessages: 301,
    committedMessages: 301,
    returnedMessages: 1,
    startIndex: 199,
    endIndex: 200,
    hasMoreBefore: false,
    nextBeforeIndex: null,
  });
  const h = createWindowHarness({ olderPages: [older] });
  const oldRequest = await establishWindow(h, recent, 300, 301);

  await h.mirror.loadOlderThreadHistoryPage(THREAD_ID);
  const pending = h.starts.at(-1);
  assert.equal(pending.renderFloor, 100);

  h.mirror.notifyStreamEvent({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    requestId: oldRequest.requestId,
    events: [],
    renderState: renderState(0, [], 301),
  });

  await h.mirror.startCommittedThreadStream(
    THREAD_ID,
    recent,
    `side-chat:${THREAD_ID}`,
  );
  const rebound = h.starts.at(-1);
  assert.equal(
    rebound.renderFloor,
    pending.renderFloor,
    "the still-pending target must be rebound instead of accepting the stale floor",
  );
  assert.notEqual(rebound.requestId, pending.requestId);
});

test("normal-plan fallback starts new, full-window, and invariant-satisfied threads once", async () => {
  const empty = transcript([], {
    returnedMessages: 0,
    startIndex: 0,
    endIndex: 0,
    hasMoreBefore: false,
    nextBeforeIndex: null,
  });
  const newThread = createWindowHarness();
  await newThread.mirror.startCommittedThreadStream(
    THREAD_ID,
    empty,
    SELECTED_THREAD_STREAM_CONSUMER_ID,
  );
  assert.equal(newThread.starts.length, 1);
  assert.equal(newThread.starts[0].renderFloor ?? 0, 0);
  assert.ok(newThread.starts[0].requestId);

  const fullWindow = createWindowHarness();
  fullWindow.mirror.applyRemoteTranscript(THREAD_ID, empty);
  fullWindow.mirror.ingest({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    events: [],
    renderState: renderState(0, [], 0),
  });
  await fullWindow.mirror.startCommittedThreadStream(
    THREAD_ID,
    empty,
    SELECTED_THREAD_STREAM_CONSUMER_ID,
  );
  assert.equal(fullWindow.starts.length, 1);
  assert.equal(fullWindow.starts[0].renderFloor ?? 0, 0);

  const satisfiedTranscript = transcript([message(300, "user", "loaded")], {
    totalMessages: 300,
    committedMessages: 300,
    returnedMessages: 1,
    startIndex: 299,
    endIndex: 300,
    hasMoreBefore: false,
    nextBeforeIndex: null,
  });
  const satisfied = createWindowHarness();
  await establishWindow(satisfied, satisfiedTranscript, 300, 300);
  assert.equal(satisfied.starts.length, 1);
  assert.equal(satisfied.starts[0].renderFloor, 300);
});

test("cold cache snapshot seeds effectiveFloor before reconcile and never settles pending", async () => {
  const cachedTranscript = transcript([message(150, "user", "cached body")], {
    totalMessages: 300,
    committedMessages: 300,
    returnedMessages: 1,
    startIndex: 149,
    endIndex: 150,
    hasMoreBefore: false,
    nextBeforeIndex: null,
  });
  const h = createWindowHarness();
  const first = await establishWindow(h, cachedTranscript, 300, 300);
  assert.equal(
    first.renderFloor,
    50,
    "cold floor 300 plus loaded seq 150 must expand from the seeded floor",
  );

  h.mirror.ingest({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    events: [],
    renderState: renderState(50, [], 300),
  });
  await h.mirror.startCommittedThreadStream(
    THREAD_ID,
    cachedTranscript,
    `side-chat:${THREAD_ID}`,
  );
  assert.equal(
    h.starts.at(-1).renderFloor,
    50,
    "a synthesized/cache frame may update the mirror but cannot settle pending",
  );
  assert.notEqual(h.starts.at(-1).requestId, first.requestId);
});

test("cache restore with pre-floor bodies enters the demand gate", async () => {
  const cachedTranscript = transcript([message(150, "user", "cached body")], {
    totalMessages: 300,
    committedMessages: 300,
    returnedMessages: 1,
    startIndex: 149,
    endIndex: 150,
    hasMoreBefore: false,
    nextBeforeIndex: null,
    hasMoreAfter: false,
    nextAfterIndex: null,
  });
  const caughtUp = transcript([], {
    totalMessages: 300,
    committedMessages: 300,
    returnedMessages: 0,
    startIndex: 300,
    endIndex: 300,
    hasMoreBefore: true,
    nextBeforeIndex: 300,
    hasMoreAfter: false,
    nextAfterIndex: null,
  });
  const h = createWindowHarness({
    cachedEntry: {
      transcript: cachedTranscript,
      renderState: renderState(300, [], 300),
    },
    olderPages: [caughtUp],
  });

  await h.mirror.loadSelectedThreadTranscript(THREAD_ID);
  assert.equal(h.starts.length, 1);
  assert.equal(
    h.starts[0].renderFloor,
    50,
    "cache restore must reconcile the loaded seq 150 body against floor 300",
  );
});

test("last-owner stop cancels pending so reopen reconciles with a fresh request", async () => {
  const cachedTranscript = transcript([message(150, "user", "cached body")], {
    totalMessages: 300,
    committedMessages: 300,
    returnedMessages: 1,
    startIndex: 149,
    endIndex: 150,
    hasMoreBefore: false,
    nextBeforeIndex: null,
  });
  const h = createWindowHarness();
  const pending = await establishWindow(h, cachedTranscript, 300, 300);
  assert.equal(pending.renderFloor, 50);

  await h.mirror.stopCommittedThreadStream({
    threadId: THREAD_ID,
    consumerId: SELECTED_THREAD_STREAM_CONSUMER_ID,
  });
  await h.mirror.startCommittedThreadStream(
    THREAD_ID,
    cachedTranscript,
    SELECTED_THREAD_STREAM_CONSUMER_ID,
  );
  assert.equal(h.stops.length, 1);
  assert.equal(h.starts.length, 2);
  assert.equal(h.starts[1].renderFloor, 50);
  assert.notEqual(h.starts[1].requestId, pending.requestId);
});

test("gap error cancels pending and authoritative refetch re-enters the start gate", async () => {
  const cachedTranscript = transcript([message(150, "user", "cached body")], {
    totalMessages: 300,
    committedMessages: 300,
    returnedMessages: 1,
    startIndex: 149,
    endIndex: 150,
    hasMoreBefore: false,
    nextBeforeIndex: null,
  });
  const h = createWindowHarness({ authoritativeTranscript: cachedTranscript });
  const pending = await establishWindow(h, cachedTranscript, 300, 300);

  h.mirror.notifyStreamEvent({
    type: "error",
    requestId: pending.requestId,
    threadId: THREAD_ID,
    runId: "thread-stream-gap",
    error: "Thread stream seq gap after 300; authoritative refetch required",
  });
  await flushAsync();
  await flushAsync();
  assert.deepEqual(h.fullFetches, [THREAD_ID]);
  assert.equal(h.starts.length, 2);
  assert.notEqual(h.starts[1].requestId, pending.requestId);
  assert.equal(
    h.starts[1].renderFloor,
    50,
    "refetch demand reconciles the still-loaded pre-floor body",
  );
});

test("uiMessages drives neededFloor; success preserves records and a genuine re-degrade drops them", async () => {
  const recent = transcript([
    message(3, "user", "recent question"),
    message(4, "assistant", "recent answer"),
  ]);
  const older = transcript(
    [message(1, "user", "older question"), message(2, "assistant", "older answer")],
    {
      startIndex: 0,
      endIndex: 2,
      hasMoreBefore: false,
      nextBeforeIndex: null,
    },
  );
  const h = createWindowHarness({ olderPages: [older] });
  await establishWindow(h, recent, 3, 4);
  await h.mirror.loadOlderThreadHistoryPage(THREAD_ID);
  const expansion = h.starts.at(-1);
  assert.deepEqual(
    h.mirror.getThreadSnapshot(THREAD_ID).records,
    [],
    "HTTP pagination extends uiMessages without recordsBySeq",
  );

  h.mirror.notifyStreamEvent({
    type: "committed_message",
    requestId: expansion.requestId,
    threadId: THREAD_ID,
    runId: "run-divergence",
    seq: 1,
    message: message(1, "user", "older question"),
  });
  h.mirror.notifyStreamEvent({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    requestId: expansion.requestId,
    events: [],
    renderState: renderState(0, [turnRow(1, 2), turnRow(3, 4)], 4),
  });
  assert.deepEqual(
    h.mirror.getThreadSnapshot(THREAD_ID).records.map((record) => record.seq),
    [1],
    "successful snapshot-only expansion carries no marker and drops nothing",
  );

  h.mirror.notifyStreamEvent({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    requestId: expansion.requestId,
    replay: "windowed",
    events: [],
    renderState: renderState(3, [turnRow(3, 4)], 5),
  });
  const degraded = h.mirror.getThreadSnapshot(THREAD_ID);
  assert.deepEqual(degraded.records, []);
  assert.ok(
    degraded.messages.some((entry) => entry.seq === 1),
    "record reset does not erase HTTP-loaded uiMessages; they demand re-expansion",
  );
});

test("successful prepay growth is capped and cannot jump to ledger head beyond the cap", async () => {
  const initial = transcript([message(100_000, "user", "tail")], {
    totalMessages: 100_000,
    committedMessages: 100_000,
    returnedMessages: 1,
    startIndex: 99_999,
    endIndex: 100_000,
    hasMoreBefore: false,
    nextBeforeIndex: null,
  });
  const h = createWindowHarness();
  await establishWindow(h, initial, 100_000, 100_000);

  const demands = [99_999, 99_898, 99_697, 99_296, 98_495];
  const targets = [99_899, 99_698, 99_297, 98_496, 96_895];
  for (let index = 0; index < demands.length; index += 1) {
    const needed = demands[index];
    h.mirror.applyOlderHistoryPage(
      THREAD_ID,
      transcript([message(needed, "user", `older ${needed}`)], {
        totalMessages: 100_000,
        committedMessages: 100_000,
        returnedMessages: 1,
        startIndex: needed - 1,
        endIndex: needed,
        hasMoreBefore: false,
        nextBeforeIndex: null,
      }),
    );
    await h.mirror.startCommittedThreadStream(
      THREAD_ID,
      initial,
      SELECTED_THREAD_STREAM_CONSUMER_ID,
    );
    const request = h.starts.at(-1);
    assert.equal(request.renderFloor, targets[index]);
    h.mirror.notifyStreamEvent({
      type: "thread_render_frame",
      threadId: THREAD_ID,
      requestId: request.requestId,
      events: [],
      renderState: renderState(targets[index], [], 100_000),
    });
  }

  const neededAtCap = 96_894;
  h.mirror.applyOlderHistoryPage(
    THREAD_ID,
    transcript([message(neededAtCap, "user", "cap demand")], {
      totalMessages: 100_000,
      committedMessages: 100_000,
      returnedMessages: 1,
      startIndex: neededAtCap - 1,
      endIndex: neededAtCap,
      hasMoreBefore: false,
      nextBeforeIndex: null,
    }),
  );
  await h.mirror.startCommittedThreadStream(
    THREAD_ID,
    initial,
    SELECTED_THREAD_STREAM_CONSUMER_ID,
  );
  const capped = h.starts.at(-1);
  assert.equal(
    capped.renderFloor,
    neededAtCap - 2048,
    "prepayMargin stops at MAX_RENDER_WINDOW_PREPAY_RECORDS",
  );
  assert.ok(capped.renderFloor > 0);
  h.mirror.notifyStreamEvent({
    type: "thread_render_frame",
    threadId: THREAD_ID,
    requestId: capped.requestId,
    events: [],
    renderState: renderState(capped.renderFloor, [], 100_000),
  });

  h.mirror.applyOlderHistoryPage(
    THREAD_ID,
    transcript([message(2049, "user", "just outside the cap")], {
      totalMessages: 100_000,
      committedMessages: 100_000,
      returnedMessages: 1,
      startIndex: 2048,
      endIndex: 2049,
      hasMoreBefore: false,
      nextBeforeIndex: null,
    }),
  );
  await h.mirror.startCommittedThreadStream(
    THREAD_ID,
    initial,
    SELECTED_THREAD_STREAM_CONSUMER_ID,
  );
  assert.equal(
    h.starts.at(-1).renderFloor,
    1,
    "loaded body just beyond the cap cannot speculatively request floor 0",
  );

  await h.mirror.stopCommittedThreadStream({
    threadId: THREAD_ID,
    consumerId: SELECTED_THREAD_STREAM_CONSUMER_ID,
  });
  h.mirror.applyOlderHistoryPage(
    THREAD_ID,
    transcript([message(1, "user", "ledger head")], {
      totalMessages: 100_000,
      committedMessages: 100_000,
      returnedMessages: 1,
      startIndex: 0,
      endIndex: 1,
      hasMoreBefore: false,
      nextBeforeIndex: null,
    }),
  );
  await h.mirror.startCommittedThreadStream(
    THREAD_ID,
    initial,
    SELECTED_THREAD_STREAM_CONSUMER_ID,
  );
  assert.equal(
    h.starts.at(-1).renderFloor ?? 0,
    0,
    "floor 0 is reached only once loaded bodies are within the cap of head",
  );
});
