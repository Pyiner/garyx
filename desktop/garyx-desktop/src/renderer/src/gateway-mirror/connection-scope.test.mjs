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

test("a cancelled trailing refresh settles instead of hanging forever", async () => {
  const first = deferred();
  let stateCalls = 0;
  const mirror = new GatewayMirror({
    getState: () => {
      stateCalls += 1;
      return stateCalls === 1 ? first.promise : new Promise(() => {});
    },
    listCustomAgents: async () => ({
      agents: [],
      defaultAgentId: null,
      effectiveDefaultAgentId: null,
    }),
    getThreadHistory: () => Promise.reject(new Error("unused")),
  });
  mirror.beginConnectionScope("http://gateway-a");

  const flightOne = mirror.refreshDesktopState();
  const joined = mirror.refreshDesktopState(); // requests the trailing round
  first.resolve({ entitiesGatewayUrl: "http://gateway-a" });
  await flightOne;
  await joined;
  // The trailing round is now scheduled (timer pending); a third caller
  // holds its promise.
  const trailing = mirror.refreshDesktopState();
  assert.notEqual(trailing, flightOne, "a trailing round was scheduled");

  mirror.beginConnectionScope("http://gateway-b");
  // Explicit settlement deadline: a dropped-reference regression must FAIL
  // fast here, not hang the suite until the runner cancels it.
  const outcome = await Promise.race([
    trailing.then(
      () => "resolved",
      (reason) => `rejected:${reason instanceof Error ? reason.message : reason}`,
    ),
    new Promise((resolve) => setTimeout(() => resolve("hung"), 250)),
  ]);
  assert.equal(
    outcome,
    "rejected:gateway connection scope changed",
    "the abandoned trailing promise settles with a cancellation",
  );
});

test("a refresh answer with a foreign gateway identity is never published", async () => {
  const answer = deferred();
  let stateCalls = 0;
  const mirror = new GatewayMirror({
    getState: () => {
      stateCalls += 1;
      return stateCalls === 1 ? answer.promise : new Promise(() => {});
    },
    listCustomAgents: async () => ({
      agents: [],
      defaultAgentId: null,
      effectiveDefaultAgentId: null,
    }),
    getThreadHistory: () => Promise.reject(new Error("unused")),
  });
  const aState = {
    entitiesGatewayUrl: "http://gateway-a",
    threads: [],
    sessions: [],
    automations: [],
  };
  mirror.beginConnectionScope("");
  mirror.beginConnectionScope("http://gateway-a", { desktopState: aState });
  // First landing adopted the committed root without any reset.
  assert.equal(
    mirror.getRootSnapshot().desktopState,
    aState,
    "cold start adopts the committed root",
  );

  const pending = mirror.refreshDesktopState();
  // Same epoch, but the ANSWER identifies as another gateway: the ingress
  // rejects it for React; the mirror must apply the same discipline.
  answer.resolve({ entitiesGatewayUrl: "http://gateway-b", threads: [] });
  await pending;
  assert.equal(
    mirror.getRootSnapshot().desktopState,
    aState,
    "a foreign-identity answer is handed to the caller but never published",
  );
});

test("a failed catalog refresh keeps the last-known catalog", async () => {
  let fail = false;
  const mirror = new GatewayMirror({
    getState: () => Promise.reject(new Error("unused")),
    listCustomAgents: () =>
      fail
        ? Promise.reject(new Error("gateway hiccup"))
        : Promise.resolve({
            agents: [{ id: "agent::available" }],
            defaultAgentId: "agent::available",
            effectiveDefaultAgentId: "agent::available",
          }),
    getThreadHistory: () => Promise.reject(new Error("unused")),
  });
  mirror.beginConnectionScope("http://gateway-a");
  await mirror.refreshAgentCatalog();
  assert.equal(mirror.getCatalogSnapshot().agents[0].id, "agent::available");

  // A transient failure must never map to an EMPTY catalog over good data
  // (that would lock the composer behind "Enable an agent...").
  fail = true;
  await mirror.refreshAgentCatalog();
  assert.equal(
    mirror.getCatalogSnapshot().agents[0]?.id,
    "agent::available",
    "the last-known catalog survives a failed refresh",
  );
});

test("an older catalog response cannot roll back a newer request's publish", async () => {
  const first = deferred();
  const second = deferred();
  let calls = 0;
  const mirror = new GatewayMirror({
    getState: () => Promise.reject(new Error("unused")),
    listCustomAgents: () => {
      calls += 1;
      return calls === 1 ? first.promise : second.promise;
    },
    getThreadHistory: () => Promise.reject(new Error("unused")),
  });
  mirror.beginConnectionScope("http://gateway-a");
  const older = mirror.refreshAgentCatalog();
  const newer = mirror.refreshAgentCatalog();
  // The NEWER request answers first; the older answer arrives later and
  // must not roll the catalog back (same epoch, so only the monotonic
  // request order can protect this).
  second.resolve({
    agents: [{ id: "agent::new" }],
    defaultAgentId: "agent::new",
    effectiveDefaultAgentId: "agent::new",
  });
  await newer;
  first.resolve({
    agents: [{ id: "agent::old" }],
    defaultAgentId: "agent::old",
    effectiveDefaultAgentId: "agent::old",
  });
  await older;
  assert.equal(
    mirror.getCatalogSnapshot().agents[0]?.id,
    "agent::new",
    "the latest issued request owns the catalog",
  );
});

test("catalog is a request state machine: failures publish, never blank silently", async () => {
  const first = deferred();
  const second = deferred();
  let calls = 0;
  const mirror = new GatewayMirror({
    getState: () => Promise.reject(new Error("unused")),
    listCustomAgents: () => {
      calls += 1;
      return calls === 1 ? first.promise : second.promise;
    },
    getThreadHistory: () => Promise.reject(new Error("unused")),
  });
  mirror.beginConnectionScope("http://gateway-a");
  assert.equal(mirror.getCatalogSnapshot().phase, "loading");

  // Reviewer counter-example: the NEWER request fails, the OLDER one
  // succeeds afterwards. Take-latest drops the older success, but the
  // latest failure must be PUBLISHED — never a silent empty catalog.
  const older = mirror.refreshAgentCatalog();
  const newer = mirror.refreshAgentCatalog();
  second.reject(new Error("gateway hiccup"));
  assert.equal(await newer, false);
  assert.equal(mirror.getCatalogSnapshot().phase, "error");
  first.resolve({
    agents: [{ id: "agent::old" }],
    defaultAgentId: "agent::old",
    effectiveDefaultAgentId: "agent::old",
  });
  await older;
  assert.equal(
    mirror.getCatalogSnapshot().phase,
    "error",
    "the latest request's failure stays published",
  );

  // A later successful refresh recovers to ready with content (fresh
  // mirror models the recovery round).
  const freshOk = new GatewayMirror({
    getState: () => Promise.reject(new Error("unused")),
    listCustomAgents: async () => ({
      agents: [{ id: "agent::fresh" }],
      defaultAgentId: "agent::fresh",
      effectiveDefaultAgentId: "agent::fresh",
    }),
    getThreadHistory: () => Promise.reject(new Error("unused")),
  });
  freshOk.beginConnectionScope("http://gateway-a");
  await freshOk.refreshAgentCatalog();
  assert.equal(freshOk.getCatalogSnapshot().phase, "ready");
  assert.equal(freshOk.getCatalogSnapshot().agents[0].id, "agent::fresh");
});

test("a background refresh never unsettles a pending foreground owner", async () => {
  const foreground = deferred();
  let calls = 0;
  const mirror = new GatewayMirror({
    getState: () => Promise.reject(new Error("unused")),
    listCustomAgents: () => {
      calls += 1;
      return foreground.promise;
    },
    getThreadHistory: () => Promise.reject(new Error("unused")),
  });
  mirror.beginConnectionScope("http://gateway-a");

  // Foreground load in flight; a focus revalidation arrives.
  const fg = mirror.refreshAgentCatalog();
  assert.equal(mirror.getCatalogSnapshot().phase, "loading");
  // Deadline race: a regression that makes the background call join (or
  // cancel) the pending foreground request must FAIL fast, not hang the
  // suite until the runner cancels it.
  const bg = await Promise.race([
    mirror.refreshAgentCatalog({ background: true }),
    new Promise((resolve) => setTimeout(() => resolve("hung"), 250)),
  ]);
  assert.equal(bg, true, "background coalesces while foreground is pending");
  assert.equal(calls, 1, "no second request is issued");

  // The foreground answer still owns its publish (the reviewer's probe:
  // a background ordinal advance used to cancel it and park the phase at
  // loading forever).
  foreground.resolve({
    agents: [{ id: "agent::fg" }],
    defaultAgentId: "agent::fg",
    effectiveDefaultAgentId: "agent::fg",
  });
  await fg;
  assert.equal(mirror.getCatalogSnapshot().phase, "ready");
  assert.equal(mirror.getCatalogSnapshot().agents[0].id, "agent::fg");
});

test("a background failure after ready keeps content; success publishes", async () => {
  let mode = "ok";
  const mirror = new GatewayMirror({
    getState: () => Promise.reject(new Error("unused")),
    listCustomAgents: () =>
      mode === "ok"
        ? Promise.resolve({
            agents: [{ id: "agent::ready" }],
            defaultAgentId: "agent::ready",
            effectiveDefaultAgentId: "agent::ready",
          })
        : Promise.reject(new Error("transient")),
    getThreadHistory: () => Promise.reject(new Error("unused")),
  });
  mirror.beginConnectionScope("http://gateway-a");
  await mirror.refreshAgentCatalog();
  assert.equal(mirror.getCatalogSnapshot().phase, "ready");

  mode = "fail";
  assert.equal(await mirror.refreshAgentCatalog({ background: true }), false);
  assert.equal(
    mirror.getCatalogSnapshot().phase,
    "ready",
    "a transient background failure never disturbs ready content",
  );
  assert.equal(mirror.getCatalogSnapshot().agents[0].id, "agent::ready");

  mode = "ok2";
  // (mode not matching "ok" -> rejection; flip back for a real publish)
  mode = "ok";
  await mirror.refreshAgentCatalog({ background: true });
  assert.equal(mirror.getCatalogSnapshot().phase, "ready");
});

test("a switch REQUEST already fences the selected-thread load (pre-commit window)", async () => {
  // The reviewer's BLOCKER window: settings for gateway B are saved (the
  // ingress domain generation advances at the switch REQUEST), transport
  // now answers with B's history, but B's state has NOT committed (mirror
  // epoch unchanged, mirror still persists under A's scope). The
  // continuation lease must already be dead.
  const { PinnedOrderIngress, installPinnedOrderIngress } = await import(
    "../pinned-order-ingress.ts"
  );
  const ingress = new PinnedOrderIngress("renderer-window-probe");
  installPinnedOrderIngress(ingress);
  ingress.beginGatewaySwitch("https://gateway-a.test");

  const history = deferred();
  const persists = [];
  const mirror = new GatewayMirror({
    getState: () => Promise.reject(new Error("unused")),
    listCustomAgents: async () => ({
      agents: [],
      defaultAgentId: null,
      effectiveDefaultAgentId: null,
    }),
    getThreadHistory: () => Promise.reject(new Error("unused")),
    getThreadHistoryFull: () => history.promise,
    loadThreadTranscriptCache: async () => null,
    clearThreadTranscriptCache: async () => {},
    startThreadStream: async () => {},
    stopThreadStream: async () => {},
    saveThreadTranscriptCache: async (scope, transcript) => {
      persists.push({ scope, text: transcript.messages[0]?.text });
    },
  });
  mirror.beginConnectionScope("http://gateway-a");
  mirror.setTranscriptLifecycleDeps({
    setHistoryLoading: () => {},
    setError: () => {},
    requestSelectedThreadMessagesBottomSnap: () => {},
    setPendingAutomationRun: () => {},
    transcriptHasAutomationResponse: () => false,
    scheduleHistoryRefresh: () => {},
    scheduleDesktopStateRefresh: () => {},
    selectedThreadGenerationRef: { current: 0 },
    lastRenderedMessageThreadRef: { current: null },
    selectedThreadIdRef: { current: THREAD },
    sideChatThreadIdRef: { current: null },
    sideChatStreamConsumerId: (threadId) => `side-chat:${threadId}`,
    messagesRef: { current: null },
    pendingMessagesPrependAnchorRef: { current: null },
  });

  const pending = mirror.loadSelectedThreadTranscript(THREAD);
  // Switch REQUEST (generation advances); NO commit (epoch unchanged).
  ingress.beginGatewaySwitch("https://gateway-b.test");
  history.resolve(transcriptWith("B secret"));
  await pending.catch(() => {});

  assert.equal(
    mirror.getThreadSnapshot(THREAD).messages.length,
    0,
    "B's answer does not enter the still-A mirror",
  );
  assert.deepEqual(persists, [], "nothing persists under gateway A's scope");
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

test("a late older-page from another thread leaves the successor's anchor alone", async () => {
  const history = deferred();
  const mirror = new GatewayMirror({
    getState: () => Promise.reject(new Error("unused")),
    listCustomAgents: async () => ({
      agents: [],
      defaultAgentId: null,
      effectiveDefaultAgentId: null,
    }),
    getThreadHistory: () => history.promise,
    startThreadStream: async () => {},
    stopThreadStream: async () => {},
  });
  mirror.beginConnectionScope("http://gateway-a");
  mirror.applyRemoteTranscript(
    THREAD,
    transcriptWith("A recent", {
      pageInfo: { hasMoreBefore: true, nextBeforeIndex: 42 },
    }),
    { persist: false },
  );
  const anchorRef = { current: null };
  const selectedRef = { current: THREAD };
  const errors = [];
  mirror.setTranscriptLifecycleDeps({
    messagesRef: { current: null },
    pendingMessagesPrependAnchorRef: anchorRef,
    selectedThreadIdRef: selectedRef,
    setError: (error) => {
      if (error) {
        errors.push(error);
      }
    },
  });

  const pending = mirror.loadOlderThreadHistoryPage(THREAD);
  // Same gateway: the user switches to ANOTHER thread which captures its
  // own prepend anchor while A's page is still in flight.
  selectedRef.current = "thread::b-side";
  anchorRef.current = {
    threadId: "thread::b-side",
    scrollHeight: 100,
    scrollTop: 50,
  };
  history.reject(new Error("gateway a page failed"));
  await pending;

  assert.deepEqual(
    anchorRef.current,
    { threadId: "thread::b-side", scrollHeight: 100, scrollTop: 50 },
    "B's anchor survives A's late failure",
  );
  assert.deepEqual(errors, [], "A's page error is not surfaced on B");
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
