// Recorded-ack replay harness for the dispatch orchestrator (endgame
// architecture batch 3c-2).
//
// Dual-run contract: every scenario replays the same scripted gateway
// responses (shaped like the real OpenChatStreamResult /
// SendStreamingInputResult wire types) into two bindings of the SAME
// orchestration code and asserts identical effect traces:
//   A. "legacy-shaped" binding — a standalone message-machine reducer with
//      a ref shadow (the pre-3a useReducer semantics) and the pre-3c-1
//      legacy live-stream Record store.
//   B. "mirror" binding — a real GatewayMirror instance: machine dispatch
//      through mirror.dispatchMachineAction (the AppShell proxy shape),
//      live-stream through the 3c-1 mirror proxies, orchestration through
//      the mirror facade methods.
// No live gateway is involved: dispatch has real side effects, so the
// design mandates replaying recorded responses, never double-sending.
//
// Traces mask wall-clock timestamps and crypto.randomUUID() suffixes —
// the orchestration stamps both (legacy behavior kept verbatim), so they
// legitimately differ between the two runs.

import assert from "node:assert/strict";
import { test } from "node:test";

import { DispatchOrchestrator } from "./dispatch-orchestrator.ts";
import { collectTerminalThreadIntents } from "./dispatch-machine.ts";
import { GatewayMirror } from "./mirror.ts";
import {
  initialMessageMachineState,
  messageMachineReducer,
} from "../message-machine.ts";
import { userMessageIdForOrigin } from "./transcript-materialize.ts";

const THREAD_ID = "thread::orchestrator-replay";
const GATEWAY_URL = "http://127.0.0.1:31337";

function testIntent(id, overrides = {}) {
  return {
    intentId: `intent:${id}`,
    threadId: THREAD_ID,
    text: overrides.text ?? `prompt ${id}`,
    images: [],
    files: [],
    createdAt: "2026-07-04T10:00:00.000Z",
    updatedAt: "2026-07-04T10:00:00.000Z",
    state: overrides.state ?? "dispatch_requested",
    source: overrides.source ?? "composer_send",
    dispatchMode: overrides.dispatchMode ?? "sync_send",
    ...overrides,
  };
}

function threadSummary(threadId) {
  return {
    id: threadId,
    title: "Test Thread",
    updatedAt: "2026-07-04T10:00:05.000Z",
  };
}

function connectionStatus() {
  return {
    ok: true,
    bridgeReady: true,
    gatewayUrl: GATEWAY_URL,
  };
}

// Mask non-deterministic parts so the two runs compare structurally.
function masked(value) {
  return JSON.parse(
    JSON.stringify(value, (key, entry) => {
      if (
        (key === "timestamp" || key === "createdAt" || key === "updatedAt") &&
        typeof entry === "string"
      ) {
        return "<time>";
      }
      if (typeof entry === "string") {
        return entry.replace(
          /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g,
          "<uuid>",
        );
      }
      return entry;
    }),
  );
}

function replayMachineActions(trace, { reclaimReleasedIntents = false } = {}) {
  let state = initialMessageMachineState;
  for (const entry of trace) {
    if (entry.kind !== "action") continue;
    state = messageMachineReducer(state, entry.action);
    if (reclaimReleasedIntents && entry.action.type === "thread/clear") {
      // This harness does not seed transcript-owned local references into the
      // real mirror, so every terminal intent is reclaimable at release time.
      state = collectTerminalThreadIntents(
        state,
        entry.action.threadId,
        new Set(),
      );
    }
  }
  return state;
}

/**
 * Build one side's deps + trace around the shared scripted IPC. `bindings`
 * supplies the side-specific machine dispatch and live-stream storage.
 */
function makeSide(name, script, options = {}) {
  const trace = [];
  const record = (kind, payload) => {
    trace.push(masked({ kind, ...payload }));
  };

  const messageStateRef = { current: initialMessageMachineState };
  const messagesByThreadRef = { current: {} };
  const desktop = {
    threads: [],
    sessions: [],
  };

  let dispatchMachineAction;
  let liveStream;
  let mirror = null;
  if (name === "mirror") {
    mirror = new GatewayMirror();
    dispatchMachineAction = (action) => {
      messageStateRef.current = mirror.dispatchMachineAction(action);
    };
    const liveStreamRef = { current: {} };
    liveStream = {
      ref: liveStreamRef,
      update: (threadId, updater) => {
        const next = mirror.updateThreadLiveStream(threadId, updater);
        liveStreamRef.current = mirror.getLiveStreamMap();
        return next;
      },
      clear: (threadId) => {
        mirror.clearThreadLiveStream(threadId);
        liveStreamRef.current = mirror.getLiveStreamMap();
      },
      get: (threadId) => liveStreamRef.current[threadId] || null,
    };
  } else {
    dispatchMachineAction = (action) => {
      messageStateRef.current = messageMachineReducer(
        messageStateRef.current,
        action,
      );
    };
    const liveStreamRef = { current: {} };
    liveStream = {
      ref: liveStreamRef,
      update: (threadId, updater) => {
        const next = updater(liveStreamRef.current[threadId] || null);
        const updated = { ...liveStreamRef.current };
        if (next) {
          updated[threadId] = next;
        } else {
          delete updated[threadId];
        }
        liveStreamRef.current = updated;
        return next;
      },
      clear: (threadId) => {
        liveStream.update(threadId, () => null);
      },
      get: (threadId) => liveStreamRef.current[threadId] || null,
    };
  }

  const dispatchMessageState = (action) => {
    record("action", { action });
    dispatchMachineAction(action);
  };

  // 6b-2d: the machine/live-stream/message/accept surface moved off the
  // deps and onto the MirrorPort; both sides get a RECORDING port so the
  // dual-run trace comparison stays step-by-step symmetric.
  const port = {
    dispatchMachineAction(action) {
      record("action", { action });
      dispatchMachineAction(action);
      return messageStateRef.current;
    },
    getMachineState: () => messageStateRef.current,
    setThreadRuntimeState(threadId, runtimeState, opts) {
      port.dispatchMachineAction({
        type: "thread/runtime",
        threadId,
        runtimeState,
        activeIntentId: opts?.activeIntentId,
        remoteRunId: opts?.remoteRunId,
        error: opts?.error,
      });
    },
    hasPendingHistoryIntents: (threadId) =>
      Object.values(messageStateRef.current.intentsById).some((intent) => {
        return (
          intent.threadId === threadId &&
          [
            "remote_accepted",
            "awaiting_provider_ack",
            "awaiting_history",
            "awaiting_response",
            "dispatching",
          ].includes(intent.state)
        );
      }),
    updateThreadLiveStream(threadId, updater) {
      const next = liveStream.update(threadId, updater);
      record("liveStream", {
        threadId,
        status: next?.streamStatus ?? null,
        runId: next?.runId ?? null,
        pendingAck: next?.pendingAckIntentIds ?? null,
      });
      return next;
    },
    getThreadLiveStream: (threadId) => liveStream.get(threadId),
    updateMessagesByThread(updater) {
      const next = updater(messagesByThreadRef.current);
      messagesByThreadRef.current = next;
      record("messages", { map: next });
      return next;
    },
    getThreadSnapshot: (threadId) => ({
      messages: messagesByThreadRef.current[threadId] || [],
      threadInfo: null,
    }),
    acceptAuthoritativeTranscript(threadId, transcript) {
      record("applyCanonicalTranscript", {
        threadId,
        messageCount: transcript.messages.length,
      });
      // Mimic the production reconciliation the transcript lifecycle runs
      // synchronously inside acceptAuthoritativeTranscript: echoed intents
      // complete and the settled thread runtime clears (markIntentsFromHistory
      // + thread/clear). Without this, the drain's busy check would stop
      // after the first queued send.
      for (const message of transcript.messages) {
        if (typeof message.id === "string" && message.id.startsWith("origin:")) {
          port.dispatchMachineAction({
            type: "intent/completed",
            intentId: message.id.slice("origin:".length),
          });
        }
      }
      port.dispatchMachineAction({ type: "thread/clear", threadId });
    },
    getThreadTitleOverrides: () => options.titleOverrides || {},
  };

  const deps = {
    scheduleHistoryRefresh: (threadId, attempts, delayMs, canonical) => {
      record("scheduleHistoryRefresh", { threadId, attempts, delayMs, canonical });
    },
    setDesktopState: (updater) => {
      const next = updater({ ...desktop });
      if (next) {
        desktop.threads = next.threads;
        desktop.sessions = next.sessions;
      }
      record("setDesktopState", {
        threads: (next?.threads || []).map((t) => `${t.id}|${t.title}`),
      });
    },
    setConnection: (status) => {
      record("setConnection", { status });
    },
    setError: (error) => {
      record("setError", { error });
    },
    recordGatewayStatusObservation: (status, reason) => {
      record("gatewayObservation", { status, reason });
    },
    requestMessagesBottomSnap: (threadId, forceStick) => {
      record("bottomSnap", { threadId, forceStick });
    },
    sideChatThreadIdsRef: { current: options.sideChatThreadIds || new Set() },
    connection: connectionStatus(),
    settingsDraft: { gatewayUrl: GATEWAY_URL, followUpBehavior: "queue" },
    desktopState: null,
    desktopAgents: [],
    canSteerQueuedPrompt: options.canSteerQueuedPrompt ?? true,
    inferProviderTypeForThread: () => "claude_code",
    openChatStream: async (input) => {
      record("ipc.openChatStream", { input: { threadId: input.threadId, clientIntentId: input.clientIntentId, message: input.message } });
      const step = script.openChatStream.shift();
      if (!step) {
        throw new Error("script exhausted: openChatStream");
      }
      if (step.error) {
        throw new Error(step.error);
      }
      return step.result;
    },
    sendStreamingInput: async (input) => {
      record("ipc.sendStreamingInput", { input: { threadId: input.threadId, clientIntentId: input.clientIntentId, message: input.message } });
      const step = script.sendStreamingInput.shift();
      if (!step) {
        throw new Error("script exhausted: sendStreamingInput");
      }
      if (step.error) {
        throw new Error(step.error);
      }
      return step.result;
    },
    getThreadHistory: async (threadId) => {
      record("ipc.getThreadHistory", { threadId });
      const step = script.getThreadHistory.shift();
      if (!step) {
        throw new Error("script exhausted: getThreadHistory");
      }
      return step.result;
    },
    interruptThread: async (threadId) => {
      record("ipc.interruptThread", { threadId });
      return { status: "ok" };
    },
    checkConnection: async () => {
      record("ipc.checkConnection", {});
      return connectionStatus();
    },
  };

  // Both sides drive a standalone orchestrator over their recording port
  // (the "mirror" side's port forwards into a real GatewayMirror's machine
  // and live-stream storage; the legacy side replays the reducer shapes).
  const orchestrator = new DispatchOrchestrator(port);
  orchestrator.setDeps(deps);
  const run = {
    appendSeededTurn: (threadId, intent, opts) =>
      orchestrator.appendSeededTurn(threadId, intent, opts),
    sendIntentOnce: (threadId, intentId, opts) =>
      orchestrator.sendIntentOnce(threadId, intentId, opts),
    runQueuedBatch: (threadId, initialIntentId) =>
      orchestrator.runQueuedBatch(threadId, initialIntentId),
    steerQueuedIntent: (intent, opts) =>
      orchestrator.steerQueuedIntent(intent, opts),
    interruptThread: (threadId) => orchestrator.interruptThread(threadId),
  };

  return {
    deps,
    dispatchMessageState,
    liveStream,
    messageStateRef,
    messagesByThreadRef,
    run,
    trace,
  };
}

function cloneScript(script) {
  return {
    openChatStream: [...(script.openChatStream || [])],
    sendStreamingInput: [...(script.sendStreamingInput || [])],
    getThreadHistory: [...(script.getThreadHistory || [])],
  };
}

/** Run one scenario against both bindings and assert identical traces. */
async function dualRun(script, scenario, options = {}) {
  const legacy = makeSide("legacy", cloneScript(script), options);
  const mirror = makeSide("mirror", cloneScript(script), options);
  const legacyResult = await scenario(legacy);
  const mirrorResult = await scenario(mirror);

  assert.deepEqual(
    mirror.trace,
    legacy.trace,
    "mirror trace must equal the legacy-shaped trace",
  );
  assert.deepEqual(masked(mirrorResult), masked(legacyResult));
  assert.deepEqual(
    masked(mirror.messageStateRef.current),
    masked(
      replayMachineActions(mirror.trace, { reclaimReleasedIntents: true }),
    ),
    "mirror state must match desktop release semantics",
  );
  assert.deepEqual(
    masked(legacy.messageStateRef.current),
    masked(replayMachineActions(legacy.trace)),
    "legacy state must match the canonical reducer",
  );
  assert.deepEqual(
    masked(mirror.liveStream.ref.current),
    masked(legacy.liveStream.ref.current),
    "final live-stream maps must match",
  );
  return { legacy, mirror, result: mirrorResult };
}

function acceptedResult(runId = "run-accepted-1") {
  return {
    runId,
    threadId: THREAD_ID,
    response: "",
    status: "accepted",
    thread: threadSummary(THREAD_ID),
  };
}

function completedResult(response, runId = "run-completed-1") {
  return {
    runId,
    threadId: THREAD_ID,
    response,
    status: "completed",
    thread: threadSummary(THREAD_ID),
  };
}

function echoTranscript(intent, response) {
  const messages = [
    {
      id: userMessageIdForOrigin(intent.intentId),
      role: "user",
      text: intent.text,
      timestamp: "2026-07-04T10:00:06.000Z",
    },
  ];
  if (response) {
    messages.push({
      id: `${THREAD_ID}:1`,
      role: "assistant",
      text: response,
      timestamp: "2026-07-04T10:00:07.000Z",
    });
  }
  return { threadId: THREAD_ID, messages };
}

test("dual-run: accepted send streams and records the run id", async () => {
  const intent = testIntent("accepted-1");
  const { mirror } = await dualRun(
    { openChatStream: [{ result: acceptedResult() }] },
    async (side) => {
      side.dispatchMessageState({ type: "intent/created", intent, enqueue: false });
      return side.run.sendIntentOnce(THREAD_ID, intent.intentId);
    },
  );

  const kinds = mirror.trace.map((entry) => entry.kind);
  assert.ok(kinds.includes("ipc.openChatStream"));
  const actionTypes = mirror.trace
    .filter((entry) => entry.kind === "action")
    .map((entry) => entry.action.type);
  assert.deepEqual(
    actionTypes.slice(0, 4),
    [
      "intent/created",
      "intent/dispatch-started",
      "intent/awaiting-response",
      "thread/runtime",
    ],
    "dispatch prologue",
  );
  assert.ok(actionTypes.includes("intent/remote-accepted"));
  // The stream stays live after an accepted result: no clear.
  assert.equal(
    mirror.liveStream.ref.current[THREAD_ID]?.runId,
    "run-accepted-1",
  );
  const refresh = mirror.trace.find(
    (entry) => entry.kind === "scheduleHistoryRefresh",
  );
  assert.deepEqual(refresh, {
    kind: "scheduleHistoryRefresh",
    threadId: THREAD_ID,
    attempts: 2,
    delayMs: 1200,
    canonical: false,
  });
});

test("dual-run: sync completion applies the canonical echo and clears the stream", async () => {
  const intent = testIntent("sync-1");
  const response = "final answer";
  const { mirror, result } = await dualRun(
    {
      openChatStream: [{ result: completedResult(response) }],
      getThreadHistory: [{ result: echoTranscript(intent, response) }],
    },
    async (side) => {
      side.dispatchMessageState({ type: "intent/created", intent, enqueue: false });
      return side.run.sendIntentOnce(THREAD_ID, intent.intentId);
    },
  );

  assert.equal(result, true);
  assert.ok(
    mirror.trace.some((entry) => entry.kind === "applyCanonicalTranscript"),
    "canonical echo transcript applies",
  );
  assert.equal(mirror.liveStream.ref.current[THREAD_ID], undefined);
});

test("dual-run: sync completion without an echo schedules a canonical refresh", async () => {
  const intent = testIntent("sync-miss-1");
  const { mirror } = await dualRun(
    {
      openChatStream: [{ result: completedResult("late answer") }],
      getThreadHistory: [
        { result: { threadId: THREAD_ID, messages: [] } },
      ],
    },
    async (side) => {
      side.dispatchMessageState({ type: "intent/created", intent, enqueue: false });
      return side.run.sendIntentOnce(THREAD_ID, intent.intentId);
    },
  );

  assert.ok(
    !mirror.trace.some((entry) => entry.kind === "applyCanonicalTranscript"),
  );
  const refresh = mirror.trace.filter(
    (entry) => entry.kind === "scheduleHistoryRefresh",
  );
  assert.deepEqual(refresh, [
    {
      kind: "scheduleHistoryRefresh",
      threadId: THREAD_ID,
      attempts: 4,
      delayMs: 1200,
      canonical: true,
    },
  ]);
});

test("dual-run: disconnected result records a gateway observation", async () => {
  const intent = testIntent("disc-1");
  const { mirror } = await dualRun(
    {
      openChatStream: [
        {
          result: {
            ...completedResult(""),
            status: "disconnected",
            runId: "run-disc-1",
          },
        },
      ],
      getThreadHistory: [
        { result: { threadId: THREAD_ID, messages: [] } },
      ],
    },
    async (side) => {
      side.dispatchMessageState({ type: "intent/created", intent, enqueue: false });
      return side.run.sendIntentOnce(THREAD_ID, intent.intentId);
    },
  );

  const observation = mirror.trace.find(
    (entry) => entry.kind === "gatewayObservation",
  );
  assert.ok(observation, "disconnected result must observe gateway status");
  assert.equal(observation.status.error, "stream disconnected");
});

test("dual-run: transient transport failure reconciles instead of failing", async () => {
  const intent = testIntent("transport-1");
  const { mirror, result } = await dualRun(
    { openChatStream: [{ error: "fetch failed" }] },
    async (side) => {
      side.dispatchMessageState({ type: "intent/created", intent, enqueue: false });
      return side.run.sendIntentOnce(THREAD_ID, intent.intentId);
    },
  );

  assert.equal(result, true, "transport drops report success to the drain");
  const actionTypes = mirror.trace
    .filter((entry) => entry.kind === "action")
    .map((entry) => entry.action.type);
  assert.ok(actionTypes.includes("intent/awaiting-history"));
  assert.ok(!actionTypes.includes("intent/failed"));
  const refresh = mirror.trace.find(
    (entry) => entry.kind === "scheduleHistoryRefresh",
  );
  assert.equal(refresh.attempts, 5);
  assert.equal(refresh.canonical, true);
});

test("dual-run: hard failure fails the intent and appends the error bubble", async () => {
  const intent = testIntent("hard-1");
  const { mirror, result } = await dualRun(
    { openChatStream: [{ error: "provider not ready: claude_code" }] },
    async (side) => {
      side.dispatchMessageState({ type: "intent/created", intent, enqueue: false });
      return side.run.sendIntentOnce(THREAD_ID, intent.intentId);
    },
  );

  assert.equal(result, false);
  const actionTypes = mirror.trace
    .filter((entry) => entry.kind === "action")
    .map((entry) => entry.action.type);
  assert.ok(actionTypes.includes("intent/failed"));
  const errorSet = mirror.trace
    .filter((entry) => entry.kind === "setError" && entry.error !== null)
    .at(-1);
  assert.match(errorSet.error, /Claude Code is not ready/);
  const finalMessages =
    mirror.messagesByThreadRef.current[THREAD_ID] || [];
  assert.ok(
    finalMessages.some(
      (entry) => entry.role === "assistant" && entry.error === true,
    ),
    "error bubble lands in the messages map",
  );
  assert.equal(mirror.liveStream.ref.current[THREAD_ID], undefined);
});

test("dual-run: interrupted send marks the intent interrupted", async () => {
  const intent = testIntent("interrupted-send-1");
  const { mirror, result } = await dualRun(
    { openChatStream: [{ error: "request interrupted" }] },
    async (side) => {
      side.dispatchMessageState({ type: "intent/created", intent, enqueue: false });
      return side.run.sendIntentOnce(THREAD_ID, intent.intentId);
    },
  );

  assert.equal(result, false);
  const actionTypes = mirror.trace
    .filter((entry) => entry.kind === "action")
    .map((entry) => entry.action.type);
  assert.ok(actionTypes.includes("intent/interrupted"));
  assert.ok(!actionTypes.includes("intent/failed"));
});

test("dual-run: runQueuedBatch drains two queued intents and clears the thread", async () => {
  const first = testIntent("queue-1", { state: "queued_local", source: "composer_queue" });
  const second = testIntent("queue-2", { state: "queued_local", source: "composer_queue" });
  const { mirror } = await dualRun(
    {
      openChatStream: [
        { result: completedResult("answer one", "run-q1") },
        { result: completedResult("answer two", "run-q2") },
      ],
      getThreadHistory: [
        { result: echoTranscript(first, "answer one") },
        { result: echoTranscript(second, "answer two") },
      ],
    },
    async (side) => {
      side.dispatchMessageState({ type: "intent/created", intent: first, enqueue: true });
      side.dispatchMessageState({ type: "intent/created", intent: second, enqueue: true });
      await side.run.runQueuedBatch(THREAD_ID);
      return null;
    },
  );

  const sends = mirror.trace.filter(
    (entry) => entry.kind === "ipc.openChatStream",
  );
  assert.equal(sends.length, 2, "both queued intents dispatch");
  assert.deepEqual(
    sends.map((entry) => entry.input.clientIntentId),
    [first.intentId, second.intentId],
    "queue order preserved",
  );
  const actionTypes = mirror.trace
    .filter((entry) => entry.kind === "action")
    .map((entry) => entry.action.type);
  assert.ok(actionTypes.includes("thread/clear"));
  assert.ok(
    mirror.trace.some((entry) => entry.kind === "ipc.checkConnection"),
    "drain finally checks the connection",
  );
});

test("dual-run: a failing queued intent requeues to the front and stops the drain", async () => {
  const first = testIntent("requeue-1", { state: "queued_local", source: "composer_queue" });
  const second = testIntent("requeue-2", { state: "queued_local", source: "composer_queue" });
  const { mirror } = await dualRun(
    {
      openChatStream: [{ error: "provider not ready: claude_code" }],
    },
    async (side) => {
      side.dispatchMessageState({ type: "intent/created", intent: first, enqueue: true });
      side.dispatchMessageState({ type: "intent/created", intent: second, enqueue: true });
      await side.run.runQueuedBatch(THREAD_ID);
      return null;
    },
  );

  const sends = mirror.trace.filter(
    (entry) => entry.kind === "ipc.openChatStream",
  );
  assert.equal(sends.length, 1, "drain stops after the failure");
  const requeue = mirror.trace.find(
    (entry) =>
      entry.kind === "action" && entry.action.type === "intent/requeue-front",
  );
  assert.equal(requeue.action.intentId, first.intentId);
  assert.equal(requeue.action.source, "queue_send");
});

test("dual-run: steering a queued intent tracks the provider ack", async () => {
  const intent = testIntent("steer-1", {
    state: "queued_local",
    source: "composer_queue",
  });
  const { mirror } = await dualRun(
    {
      sendStreamingInput: [
        {
          result: {
            status: "queued",
            threadId: THREAD_ID,
            clientIntentId: intent.intentId,
            pendingInputId: "pending-input-9",
          },
        },
      ],
    },
    async (side) => {
      side.dispatchMessageState({ type: "intent/created", intent, enqueue: true });
      await side.run.steerQueuedIntent(intent);
      return null;
    },
  );

  const accepted = mirror.trace.find(
    (entry) =>
      entry.kind === "action" && entry.action.type === "intent/remote-accepted",
  );
  assert.equal(accepted.action.awaitProviderAck, true);
  assert.equal(accepted.action.pendingInputId, "pending-input-9");
  const live = mirror.liveStream.ref.current[THREAD_ID];
  assert.deepEqual(live.pendingAckIntentIds, [intent.intentId]);
});

test("dual-run: a non-queued steer result falls back to a sync send", async () => {
  const intent = testIntent("steer-fallback-1", {
    state: "queued_local",
    source: "composer_queue",
  });
  const response = "fallback answer";
  const { mirror } = await dualRun(
    {
      sendStreamingInput: [
        {
          result: {
            status: "not_running",
            threadId: THREAD_ID,
            clientIntentId: intent.intentId,
          },
        },
      ],
      openChatStream: [{ result: completedResult(response, "run-fb") }],
      getThreadHistory: [{ result: echoTranscript(intent, response) }],
    },
    async (side) => {
      side.dispatchMessageState({ type: "intent/created", intent, enqueue: true });
      await side.run.steerQueuedIntent(intent);
      return null;
    },
  );

  assert.ok(
    mirror.trace.some((entry) => entry.kind === "ipc.openChatStream"),
    "fallback dispatches through the sync path",
  );
  const requests = mirror.trace
    .filter(
      (entry) =>
        entry.kind === "action" &&
        entry.action.type === "intent/request-dispatch",
    )
    .map((entry) => entry.action.mode);
  assert.deepEqual(requests, ["async_steer", "sync_send"]);
});

test("dual-run: a steer transport error requeues the intent", async () => {
  const intent = testIntent("steer-error-1", {
    state: "queued_local",
    source: "composer_queue",
  });
  const { mirror } = await dualRun(
    { sendStreamingInput: [{ error: "fetch failed" }] },
    async (side) => {
      side.dispatchMessageState({ type: "intent/created", intent, enqueue: true });
      await side.run.steerQueuedIntent(intent);
      return null;
    },
  );

  const requeue = mirror.trace.find(
    (entry) =>
      entry.kind === "action" && entry.action.type === "intent/requeue-front",
  );
  assert.equal(requeue.action.source, "queue_steer");
  assert.equal(requeue.action.error, "fetch failed");
  const live = mirror.liveStream.ref.current[THREAD_ID];
  assert.deepEqual(live.pendingAckIntentIds, []);
});

test("dual-run: steering is a no-op when steering is not allowed or the intent left the queue", async () => {
  const intent = testIntent("steer-guard-1", {
    state: "queued_local",
    source: "composer_queue",
  });
  await dualRun(
    {},
    async (side) => {
      side.dispatchMessageState({ type: "intent/created", intent, enqueue: true });
      await side.run.steerQueuedIntent(intent, { canSteer: false });
      await side.run.steerQueuedIntent(testIntent("steer-guard-2"), {
        canSteer: true,
      });
      return null;
    },
  );
});

test("dual-run: interrupting a busy thread marks intents and clears the stream", async () => {
  const intent = testIntent("interrupt-busy-1");
  const { mirror } = await dualRun(
    { openChatStream: [{ result: acceptedResult("run-int-1") }] },
    async (side) => {
      side.dispatchMessageState({ type: "intent/created", intent, enqueue: false });
      await side.run.sendIntentOnce(THREAD_ID, intent.intentId);
      await side.run.interruptThread(THREAD_ID);
      return null;
    },
  );

  assert.ok(
    mirror.trace.some((entry) => entry.kind === "ipc.interruptThread"),
  );
  const actionTypes = mirror.trace
    .filter((entry) => entry.kind === "action")
    .map((entry) => entry.action.type);
  assert.ok(actionTypes.includes("intent/interrupted"));
  assert.ok(actionTypes.includes("thread/clear"));
  assert.equal(mirror.liveStream.ref.current[THREAD_ID], undefined);
  const refresh = mirror.trace
    .filter((entry) => entry.kind === "scheduleHistoryRefresh")
    .at(-1);
  assert.equal(refresh.attempts, 2);
  assert.equal(refresh.delayMs, 500);
});

test("dual-run: interrupting an idle thread still reaches the gateway", async () => {
  const { mirror } = await dualRun(
    {},
    async (side) => {
      await side.run.interruptThread(THREAD_ID);
      return null;
    },
  );

  assert.ok(
    mirror.trace.some((entry) => entry.kind === "ipc.interruptThread"),
  );
  const actionTypes = mirror.trace
    .filter((entry) => entry.kind === "action")
    .map((entry) => entry.action.type);
  assert.ok(!actionTypes.includes("intent/interrupted"));
  assert.ok(!actionTypes.includes("thread/clear"));
});

test("orchestrator deps-dependent methods throw before deps are attached", async () => {
  // 6b-2d: machine reads go through the constructor-injected port, so
  // queue selection works deps-less; IPC-touching entries still gate on
  // the React-fed deps.
  const orchestrator = new DispatchOrchestrator({
    getMachineState: () => initialMessageMachineState,
  });
  assert.deepEqual(orchestrator.queueIntentIdsForThread(THREAD_ID), []);
  await assert.rejects(
    () => orchestrator.interruptThread(THREAD_ID),
    { message: /dispatch deps/ },
  );
});
