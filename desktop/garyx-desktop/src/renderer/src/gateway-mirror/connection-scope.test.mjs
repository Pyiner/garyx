// Behavior tests for the mirror's gateway connection scope: the ENTIRE
// renderer data universe (transcripts, dispatch machine, live streams,
// in-flight continuations) belongs to one gateway connection. A key change
// resets every machine in place (subscriptions survive) and invalidates
// every in-flight continuation — a stale dispatch answer or history page
// can never merge the previous gateway's data into the new universe.

import assert from "node:assert/strict";
import { test } from "node:test";

import { GatewayMirror } from "./mirror.ts";

const THREAD = "thread::same-id";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function transcriptWith(text, extra = {}) {
  return {
    threadId: THREAD,
    messages: [
      {
        id: `msg:${text}`,
        role: "assistant",
        text,
      },
    ],
    ...extra,
  };
}

function seededIntent(intentId, text) {
  return {
    intentId,
    threadId: THREAD,
    text,
    images: [],
    files: [],
    source: "composer_send",
    state: "dispatch_requested",
    dispatchMode: "sync_send",
    createdAt: 1,
  };
}

function makeDispatchDeps(overrides = {}) {
  const calls = {
    setDesktopState: 0,
    setError: [],
    scheduleHistoryRefresh: 0,
  };
  const deps = {
    scheduleHistoryRefresh: () => {
      calls.scheduleHistoryRefresh += 1;
    },
    setDesktopState: () => {
      calls.setDesktopState += 1;
    },
    setConnection: () => {},
    setError: (error) => {
      calls.setError.push(error);
    },
    recordGatewayStatusObservation: () => {},
    requestMessagesBottomSnap: () => {},
    sideChatThreadIdsRef: { current: new Set() },
    connection: null,
    settingsDraft: { gatewayUrl: "http://gateway-a" },
    desktopState: null,
    desktopAgents: [],
    canSteerQueuedPrompt: false,
    inferProviderTypeForThread: () => null,
    openChatStream: () => Promise.reject(new Error("not stubbed")),
    sendStreamingInput: () => Promise.reject(new Error("not stubbed")),
    getThreadHistory: () => Promise.reject(new Error("not stubbed")),
    checkConnection: () => Promise.resolve({ ok: true }),
    interruptThread: () => Promise.resolve(),
    ...overrides,
  };
  return { deps, calls };
}

test("a gateway switch resets the whole mirror universe in place", () => {
  const mirror = new GatewayMirror();
  mirror.beginConnectionScope("http://gateway-a");

  mirror.applyRemoteTranscript(THREAD, transcriptWith("A secret transcript"), {
    persist: false,
  });
  mirror.dispatchMachineAction({
    type: "intent/created",
    intent: {
      ...seededIntent("intent::from-a", "secret A prompt"),
      state: "queued_local",
      source: "composer_queue",
    },
    enqueue: true,
  });
  mirror.updateThreadLiveStream(THREAD, () => ({
    threadId: THREAD,
    activeIntentId: "intent::from-a",
    assistantEntryId: null,
    pendingAckIntentIds: [],
    streamStatus: "streaming",
  }));

  let threadNotifications = 0;
  let machineNotifications = 0;
  mirror.subscribeThread(THREAD, () => (threadNotifications += 1));
  mirror.subscribeMachine(() => (machineNotifications += 1));
  assert.equal(
    mirror.getThreadSnapshot(THREAD).messages.length,
    1,
    "universe A holds its transcript",
  );
  assert.ok(mirror.getMachineState().intentsById["intent::from-a"]);

  mirror.beginConnectionScope("http://gateway-b");
  assert.equal(
    mirror.getThreadSnapshot(THREAD).messages.length,
    0,
    "the same thread id renders empty in universe B",
  );
  assert.deepEqual(
    mirror.getMachineState().intentsById,
    {},
    "A's queued prompt does not survive into B",
  );
  assert.deepEqual(mirror.getLiveStreamMap(), {});
  assert.ok(threadNotifications >= 1, "thread subscribers saw the reset");
  assert.ok(machineNotifications >= 1, "machine subscribers saw the reset");

  // Subscriptions survive the reset: a new-universe commit still notifies.
  const before = threadNotifications;
  mirror.applyRemoteTranscript(THREAD, transcriptWith("B transcript"), {
    persist: false,
  });
  assert.ok(threadNotifications > before, "the subscription is still wired");
  assert.equal(
    mirror.getThreadSnapshot(THREAD).messages[0].text,
    "B transcript",
  );

  // Same-key adoption is a no-op, not another reset.
  mirror.beginConnectionScope("http://gateway-b");
  assert.equal(mirror.getThreadSnapshot(THREAD).messages.length, 1);
});

test("cold start adopts the first universe without a reset", () => {
  const mirror = new GatewayMirror();
  mirror.applyRemoteTranscript(THREAD, transcriptWith("boot transcript"), {
    persist: false,
  });
  // Mount-time "" adoption, then the hydrated key: neither may reset.
  mirror.beginConnectionScope("");
  mirror.beginConnectionScope("http://gateway-a");
  assert.equal(
    mirror.getThreadSnapshot(THREAD).messages.length,
    1,
    "boot data survives the first key adoption",
  );
});

test("a stale dispatch answer cannot merge the old universe into the new one", async () => {
  const mirror = new GatewayMirror();
  mirror.beginConnectionScope("http://gateway-a");
  const opened = deferred();
  const { deps, calls } = makeDispatchDeps({
    openChatStream: () => opened.promise,
  });
  mirror.setDispatchDeps(deps);
  mirror.dispatchMachineAction({
    type: "intent/created",
    intent: seededIntent("intent::from-a", "secret A prompt"),
    enqueue: false,
  });

  const pending = mirror.sendIntentOnce(THREAD, "intent::from-a");
  // Gateway switch while the dispatch answer is in flight (the reviewer's
  // exact probe: the late `accepted` branch used to merge A's thread into
  // B's desktop state and machine).
  mirror.beginConnectionScope("http://gateway-b");
  opened.resolve({
    status: "accepted",
    runId: "run-a-1",
    threadId: THREAD,
    thread: { id: THREAD, title: "A thread" },
  });

  assert.equal(await pending, false, "the stale dispatch reports failure");
  assert.equal(calls.setDesktopState, 0, "no stale state merge");
  assert.deepEqual(calls.setError, [null], "no stale error beyond the reset");
  assert.deepEqual(
    mirror.getMachineState().intentsById,
    {},
    "no stale intent transition lands in universe B",
  );
  assert.deepEqual(mirror.getLiveStreamMap(), {}, "no stale live stream");
  assert.equal(
    mirror.getThreadSnapshot(THREAD).messages.length,
    0,
    "no stale seeded turn survives in universe B",
  );
});

test("a stale dispatch failure surfaces nothing in the new universe", async () => {
  const mirror = new GatewayMirror();
  mirror.beginConnectionScope("http://gateway-a");
  const opened = deferred();
  const { deps, calls } = makeDispatchDeps({
    openChatStream: () => opened.promise,
  });
  mirror.setDispatchDeps(deps);
  mirror.dispatchMachineAction({
    type: "intent/created",
    intent: seededIntent("intent::from-a", "secret A prompt"),
    enqueue: false,
  });

  const pending = mirror.sendIntentOnce(THREAD, "intent::from-a");
  mirror.beginConnectionScope("http://gateway-b");
  opened.reject(new Error("gateway a exploded"));

  assert.equal(await pending, false);
  assert.deepEqual(
    calls.setError,
    [null],
    "the pre-dispatch clear is the only error write; the stale failure is silent",
  );
  assert.deepEqual(mirror.getMachineState().intentsById, {});
});

test("a previous universe's stream event is dropped whole at the listener", () => {
  const mirror = new GatewayMirror();
  mirror.beginConnectionScope("http://gateway-a");
  // A stream opened under universe A embeds its epoch in the request id.
  const staleRequestId = "desktop-stream-request-e0-7";

  mirror.beginConnectionScope("http://gateway-b");
  mirror.applyRemoteTranscript(THREAD, transcriptWith("B recent"), {
    persist: false,
  });
  // The late event from A's still-draining stream: committed events
  // included, nothing may land (this is a cross-connection boundary, not
  // within-connection request supersession). The drop happens before the
  // deps/service layer, so no side effect can fire either.
  mirror.notifyStreamEvent({
    type: "committed_message",
    threadId: THREAD,
    requestId: staleRequestId,
    seq: 999,
    message: { role: "assistant", content: "A secret tail" },
  });
  assert.deepEqual(
    mirror.getThreadSnapshot(THREAD).messages.map((message) => message.text),
    ["B recent"],
    "the stale committed event does not enter B's transcript",
  );
});

test("a stale connection-status answer cannot overwrite the new universe's", async () => {
  const mirror = new GatewayMirror();
  mirror.beginConnectionScope("http://gateway-a");
  const status = deferred();
  const connections = [];
  const { deps } = makeDispatchDeps({
    checkConnection: () => status.promise,
    setConnection: (value) => {
      connections.push(value);
    },
  });
  mirror.setDispatchDeps(deps);
  mirror.dispatchMachineAction({
    type: "intent/created",
    intent: {
      ...seededIntent("intent::from-a", "secret A prompt"),
      state: "queued_local",
      source: "composer_queue",
    },
    enqueue: true,
  });

  const pending = mirror.runQueuedBatch(THREAD, "intent::from-a");
  mirror.beginConnectionScope("http://gateway-b");
  status.resolve({ ok: false, error: "gateway a is unreachable" });
  await pending;
  assert.deepEqual(
    connections,
    [],
    "A's status answer is not written into universe B",
  );
});

test("the mirror root and catalog belong to the universe too", async () => {
  // First getState call = A's in-flight refresh; later calls (the
  // transition's own repopulation against B) stay pending forever so the
  // assertion isolates A's late answer.
  const rootRefresh = deferred();
  let stateCalls = 0;
  const mirror = new GatewayMirror({
    getState: () => {
      stateCalls += 1;
      return stateCalls === 1 ? rootRefresh.promise : new Promise(() => {});
    },
    listCustomAgents: async () => ({
      agents: [{ id: "agent::from-a" }],
      defaultAgentId: "agent::from-a",
      effectiveDefaultAgentId: "agent::from-a",
    }),
    getThreadHistory: () => Promise.reject(new Error("unused")),
  });
  mirror.beginConnectionScope("http://gateway-a");

  // A's root refresh is in flight when the switch lands.
  const pending = mirror.refreshDesktopState();
  const bState = {
    entitiesGatewayUrl: "http://gateway-b",
    threads: [],
    sessions: [],
    automations: [],
  };
  mirror.beginConnectionScope("http://gateway-b", { desktopState: bState });

  // The transition ADOPTS the committed B state as the root immediately —
  // scope-key consumers (e.g. the side-chat panel) can never read A's root
  // under B.
  assert.equal(mirror.getRootSnapshot().desktopState, bState);
  assert.deepEqual(
    mirror.getCatalogSnapshot().agents,
    [],
    "A's agent catalog does not survive into B",
  );

  const aState = {
    entitiesGatewayUrl: "http://gateway-a",
    threads: [],
    sessions: [],
    automations: [],
  };
  rootRefresh.resolve(aState);
  await pending.catch(() => {});
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(
    mirror.getRootSnapshot().desktopState,
    bState,
    "A's late root refresh does not republish under B",
  );
});

test("a stale openability answer reports not-openable without vouching", async () => {
  const refresh = deferred();
  const mirror = new GatewayMirror({
    getState: () => Promise.reject(new Error("unused")),
    listCustomAgents: async () => ({
      agents: [],
      defaultAgentId: null,
      effectiveDefaultAgentId: null,
    }),
    getThreadHistory: () => Promise.reject(new Error("unused")),
  });
  mirror.beginConnectionScope("http://gateway-a");
  const errors = [];
  mirror.setTranscriptLifecycleDeps({
    desktopState: { threads: [], sessions: [], automations: [] },
    refreshDesktopState: () => refresh.promise,
    setError: (error) => {
      errors.push(error);
    },
  });

  // A's refresh resolves AFTER the switch with a state that contains the
  // colliding thread id: it must not vouch for the id in universe B.
  const pending = mirror.ensureThreadOpenable(THREAD);
  mirror.beginConnectionScope("http://gateway-b");
  refresh.resolve({
    threads: [{ id: THREAD, title: "A thread" }],
    sessions: [{ id: THREAD, title: "A thread" }],
    automations: [],
  });
  assert.equal(await pending, false, "no stale vouch across the boundary");

  // And a stale rejection is silent (no error escapes into B).
  const rejected = deferred();
  mirror.beginConnectionScope("http://gateway-c");
  mirror.setTranscriptLifecycleDeps({
    desktopState: { threads: [], sessions: [], automations: [] },
    refreshDesktopState: () => rejected.promise,
    setError: (error) => {
      errors.push(error);
    },
  });
  const pendingFailure = mirror.ensureThreadOpenable(THREAD);
  mirror.beginConnectionScope("http://gateway-d");
  rejected.reject(new Error("gateway c went away"));
  assert.equal(await pendingFailure, false, "a stale failure is a quiet no");
  assert.deepEqual(errors, [], "nothing surfaced through setError");
});

test("a stale older-history page cannot prepend into the new universe", async () => {
  const history = deferred();
  const mirror = new GatewayMirror({
    getState: () => Promise.reject(new Error("unused")),
    listCustomAgents: () => Promise.reject(new Error("unused")),
    getThreadHistory: () => history.promise,
  });
  mirror.beginConnectionScope("http://gateway-a");
  mirror.applyRemoteTranscript(
    THREAD,
    transcriptWith("A recent", {
      pageInfo: { hasMoreBefore: true, nextBeforeIndex: 42 },
    }),
    { persist: false },
  );
  assert.ok(
    mirror.getThreadSnapshot(THREAD).historyPagination?.hasMoreBefore,
    "seed established backward pagination",
  );

  const pending = mirror.fetchOlderThreadHistoryPage(THREAD);
  // Switch while the page is in flight; universe B loads its own transcript
  // for the SAME thread id.
  mirror.beginConnectionScope("http://gateway-b");
  mirror.applyRemoteTranscript(THREAD, transcriptWith("B recent"), {
    persist: false,
  });
  history.resolve(transcriptWith("A secret older"));

  assert.equal(await pending, false, "the stale page reports no apply");
  assert.deepEqual(
    mirror.getThreadSnapshot(THREAD).messages.map((message) => message.text),
    ["B recent"],
    "A's older page is not prepended into B's transcript",
  );
});
