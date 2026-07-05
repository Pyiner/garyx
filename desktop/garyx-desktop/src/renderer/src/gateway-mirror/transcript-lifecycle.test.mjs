// Dual-run contract tests for TranscriptLifecycle slice 2a (endgame batch
// 6b-2, docs/design/appshell-transcript-dissolve.md): two independent
// GatewayMirror instances — one driven through legacy-shaped bindings
// (the pre-6b-2a hook logic extracted verbatim as pure functions), one
// through the lifecycle module — replay the same sequences; the machine
// action traces, live-stream transition traces, and terminal states must
// be deep-equal.

import assert from "node:assert/strict";
import { test } from "node:test";

import { GatewayMirror } from "./mirror.ts";
import {
  applyTranscriptRunStateRecord,
  reduceTranscriptRunState,
} from "../../../shared/transcript-sync.ts";
import { findPendingAckIntentIndex, selectThreadRuntime } from "../message-machine.ts";
import { resolveIntentHistoryMatch, visibleTranscriptMessages } from "./transcript-materialize.ts";

function makeHarness() {
  const mirror = new GatewayMirror();
  const machineTrace = [];
  const liveTrace = [];
  const titleDraftSyncs = [];
  let desktopState = {
    threads: [{ id: "thread::t1", title: "old title" }],
    sessions: [],
  };
  const port = {
    dispatchMachineAction(action) {
      machineTrace.push(action);
      return mirror.dispatchMachineAction(action);
    },
    getMachineState: () => mirror.getMachineState(),
    updateThreadLiveStream(threadId, updater) {
      const next = mirror.updateThreadLiveStream(threadId, updater);
      liveTrace.push({ threadId, next: next ? { ...next } : null });
      return next;
    },
    getLiveStreamMap: () => mirror.getLiveStreamMap(),
    getThreadSnapshotTranscript: (threadId) =>
      mirror.getThreadSnapshotTranscript(threadId),
  };
  const liveStreamStateRef = { current: {} };
  const deps = {
    setDesktopState: (updater) => {
      desktopState = updater(desktopState);
    },
    syncThreadTitleDraft: (title) => titleDraftSyncs.push(title),
    requestSelectedThreadMessagesBottomSnap: () => {},
    selectedThreadIdRef: { current: "thread::t1" },
    liveStreamStateRef,
  };
  return {
    mirror,
    port,
    deps,
    machineTrace,
    liveTrace,
    titleDraftSyncs,
    getDesktopState: () => desktopState,
    liveStreamStateRef,
  };
}

// ---- Legacy-shaped bindings: the pre-6b-2a hook logic, verbatim shapes. ----
function makeLegacyBindings(h) {
  const runStateByThread = {};
  const titleOverrides = {};

  function intentForId(intentId) {
    return h.port.getMachineState().intentsById[intentId] || null;
  }
  function setThreadRuntimeState(threadId, runtimeState, options) {
    h.port.dispatchMachineAction({
      type: "thread/runtime",
      threadId,
      runtimeState,
      activeIntentId: options?.activeIntentId,
      remoteRunId: options?.remoteRunId,
      error: options?.error,
    });
  }
  function updateLiveStreamState(threadId, updater) {
    const next = h.port.updateThreadLiveStream(threadId, updater);
    h.liveStreamStateRef.current = h.port.getLiveStreamMap();
    return next;
  }
  function clearLiveStreamState(threadId) {
    updateLiveStreamState(threadId, () => null);
  }
  function getLiveStreamState(threadId) {
    return h.liveStreamStateRef.current[threadId] || null;
  }
  function hasPendingHistoryIntents(threadId) {
    return Object.values(h.port.getMachineState().intentsById).some(
      (intent) =>
        intent.threadId === threadId &&
        [
          "remote_accepted",
          "awaiting_provider_ack",
          "awaiting_history",
          "awaiting_response",
          "dispatching",
        ].includes(intent.state),
    );
  }
  function applyThreadTitleUpdate(threadId, title) {
    const nextTitle = title.trim();
    if (!threadId || !nextTitle) return;
    titleOverrides[threadId] = nextTitle;
    h.deps.setDesktopState((current) => {
      if (!current) return current;
      let changed = false;
      const updateThread = (thread) => {
        if (thread.id !== threadId || thread.title === nextTitle) return thread;
        changed = true;
        return { ...thread, title: nextTitle };
      };
      const threads = current.threads.map(updateThread);
      const sessions = current.sessions.map(updateThread);
      return changed ? { ...current, threads, sessions } : current;
    });
    if (h.deps.selectedThreadIdRef.current === threadId) {
      h.deps.syncThreadTitleDraft(nextTitle);
    }
  }
  function publishTranscriptRunState(threadId, state) {
    runStateByThread[threadId] = state;
    if (state.title) applyThreadTitleUpdate(threadId, state.title);
    const remoteRunId = state.activeRunId || undefined;
    if (state.busy) {
      const runtimeState =
        state.activity === "reconciling" ? "reconciling_history" : "running_remote";
      updateLiveStreamState(threadId, (current) => ({
        threadId,
        runId: remoteRunId || current?.runId,
        activeIntentId: current?.activeIntentId,
        assistantEntryId: current?.assistantEntryId ?? null,
        pendingAckIntentIds: current?.pendingAckIntentIds || [],
        streamStatus:
          state.activity === "reconciling" ? "reconciling" : "streaming",
      }));
      setThreadRuntimeState(threadId, runtimeState, {
        activeIntentId: getLiveStreamState(threadId)?.activeIntentId,
        remoteRunId,
      });
      return state;
    }
    if (state.terminalStatus) {
      updateLiveStreamState(threadId, (current) =>
        current
          ? {
              ...current,
              runId: current.runId || remoteRunId,
              assistantEntryId: null,
              streamStatus:
                state.terminalStatus === "interrupted"
                  ? "interrupted"
                  : "reconciling",
            }
          : null,
      );
      if (!hasPendingHistoryIntents(threadId)) {
        h.port.dispatchMachineAction({ type: "thread/clear", threadId });
        clearLiveStreamState(threadId);
      }
    }
    return state;
  }
  return {
    syncTranscriptRunState: (threadId, transcript) =>
      publishTranscriptRunState(
        threadId,
        reduceTranscriptRunState(transcript.messages),
      ),
    applyCommittedTranscriptRunState: (event) => {
      const current =
        runStateByThread[event.threadId] ||
        reduceTranscriptRunState(
          h.port.getThreadSnapshotTranscript(event.threadId)?.messages || [],
        );
      return publishTranscriptRunState(
        event.threadId,
        applyTranscriptRunStateRecord(current, event.message, {
          seq: event.seq,
        }),
      );
    },
    markIntentsFromHistory: (threadId, transcript) => {
      const visibleTranscript = visibleTranscriptMessages(transcript);
      const intents = Object.values(h.port.getMachineState().intentsById).filter(
        (intent) =>
          intent.threadId === threadId &&
          [
            "dispatching",
            "remote_accepted",
            "awaiting_provider_ack",
            "awaiting_response",
            "awaiting_history",
          ].includes(intent.state),
      );
      for (const intent of intents) {
        const match = resolveIntentHistoryMatch(intent, visibleTranscript);
        if (!match.userVisible) continue;
        if (
          match.assistantVisible ||
          (!intent.responseText && intent.dispatchMode === "async_steer")
        ) {
          h.port.dispatchMachineAction({
            type: "intent/completed",
            intentId: intent.intentId,
          });
        } else {
          h.port.dispatchMachineAction({
            type: "intent/awaiting-history",
            intentId: intent.intentId,
            responseText: intent.responseText,
          });
        }
      }
      const runtime = selectThreadRuntime(h.port.getMachineState(), threadId);
      if (runtime && !hasPendingHistoryIntents(threadId)) {
        h.port.dispatchMachineAction({ type: "thread/clear", threadId });
        const liveStream = getLiveStreamState(threadId);
        if (
          liveStream &&
          ["reconciling", "disconnected", "failed"].includes(
            liveStream.streamStatus,
          )
        ) {
          clearLiveStreamState(threadId);
        }
      }
    },
    applyUserAck: (threadId, runId, pendingInputId) => {
      let nextIntentId;
      const acknowledgedPendingInputId = pendingInputId?.trim() || "";
      updateLiveStreamState(threadId, (current) => {
        const pendingAckIntentIds = [...(current?.pendingAckIntentIds || [])];
        const matchedIndex = findPendingAckIntentIndex(
          pendingAckIntentIds,
          acknowledgedPendingInputId,
          h.port.getMachineState().intentsById,
        );
        if (matchedIndex >= 0) {
          nextIntentId = pendingAckIntentIds[matchedIndex];
          pendingAckIntentIds.splice(matchedIndex, 1);
        } else {
          nextIntentId = undefined;
        }
        const nextPendingAckIntentIds = nextIntentId
          ? pendingAckIntentIds.filter((intentId) => intentId !== nextIntentId)
          : pendingAckIntentIds;
        return current
          ? {
              ...current,
              runId,
              activeIntentId: nextIntentId || current.activeIntentId,
              assistantEntryId: null,
              pendingAckIntentIds: nextPendingAckIntentIds,
              streamStatus: "streaming",
            }
          : null;
      });
      if (nextIntentId) {
        const acknowledgedIntent = intentForId(nextIntentId);
        h.port.dispatchMachineAction({
          type: "intent/awaiting-history",
          intentId: nextIntentId,
          responseText: acknowledgedIntent?.responseText,
        });
        h.deps.requestSelectedThreadMessagesBottomSnap(threadId, true);
        setThreadRuntimeState(threadId, "running_remote", {
          activeIntentId: nextIntentId,
          remoteRunId: runId,
        });
      }
    },
    forceReleaseThreadRuntime: (threadId) => {
      const pendingStates = [
        "dispatching",
        "remote_accepted",
        "awaiting_provider_ack",
        "awaiting_response",
        "awaiting_history",
      ];
      for (const intent of Object.values(h.port.getMachineState().intentsById)) {
        if (
          intent.threadId === threadId &&
          pendingStates.includes(intent.state)
        ) {
          h.port.dispatchMachineAction({
            type: "intent/completed",
            intentId: intent.intentId,
          });
        }
      }
      h.port.dispatchMachineAction({ type: "thread/clear", threadId });
      const liveStream = getLiveStreamState(threadId);
      if (
        liveStream &&
        ["reconciling", "disconnected", "failed"].includes(
          liveStream.streamStatus,
        )
      ) {
        clearLiveStreamState(threadId);
      }
    },
  };
}

function makeLifecycleBindings(h) {
  // Drive the SAME lifecycle instance the mirror owns, but against the
  // trace-recording port: rebuild one with the port and the harness deps.
  const { TranscriptLifecycle } = require_lifecycle();
  const lifecycle = new TranscriptLifecycle(h.port);
  lifecycle.setDeps(h.deps);
  return {
    syncTranscriptRunState: (t, tr) => lifecycle.syncTranscriptRunState(t, tr),
    applyCommittedTranscriptRunState: (e) =>
      lifecycle.applyCommittedTranscriptRunState(e),
    markIntentsFromHistory: (t, tr) => lifecycle.markIntentsFromHistory(t, tr),
    applyUserAck: (t, r, p) => lifecycle.applyUserAck(t, r, p),
    forceReleaseThreadRuntime: (t) => lifecycle.forceReleaseThreadRuntime(t),
  };
}

let lifecycleModule = null;
function require_lifecycle() {
  return lifecycleModule;
}

const THREAD = "thread::t1";

function runStartMessage(runId) {
  return {
    id: `${THREAD}:0`,
    role: "control",
    kind: "control",
    text: "",
    timestamp: "2026-07-05T10:00:00Z",
    content: { control: { kind: "run_start", run_id: runId } },
  };
}

function runCompleteMessage(runId, index) {
  return {
    id: `${THREAD}:${index}`,
    role: "control",
    kind: "control",
    text: "",
    timestamp: "2026-07-05T10:00:09Z",
    content: { control: { kind: "run_complete", run_id: runId } },
  };
}

function titleMessage(index) {
  return {
    id: `${THREAD}:${index}`,
    role: "control",
    kind: "control",
    text: "",
    timestamp: "2026-07-05T10:00:01Z",
    content: { control: { kind: "title", title: "fresh title" } },
  };
}

function replaySequence(bindings, h) {
  // 1. Busy run with a title control (sync path).
  bindings.syncTranscriptRunState(THREAD, {
    threadId: THREAD,
    messages: [runStartMessage("run-1"), titleMessage(1)],
    pendingInputs: [],
    threadInfo: null,
  });
  // 2. Committed user_ack while an intent waits in pendingAckIntentIds.
  h.port.dispatchMachineAction({
    type: "intent/created",
    intent: {
      intentId: "intent-1",
      threadId: THREAD,
      state: "awaiting_provider_ack",
      text: "hello",
      dispatchMode: "steer",
      responseText: "queued response",
    },
    enqueue: false,
  });
  h.port.updateThreadLiveStream(THREAD, (current) =>
    current
      ? { ...current, pendingAckIntentIds: ["intent-1"] }
      : {
          threadId: THREAD,
          runId: "run-1",
          assistantEntryId: null,
          pendingAckIntentIds: ["intent-1"],
          streamStatus: "streaming",
        },
  );
  h.liveStreamStateRef.current = h.port.getLiveStreamMap();
  bindings.applyUserAck(THREAD, "run-1", "pending-1");
  // 3. Terminal run-complete via the committed path.
  bindings.applyCommittedTranscriptRunState({
    threadId: THREAD,
    seq: 9,
    message: runCompleteMessage("run-1", 2),
  });
  // 4. Force release with a failed live stream.
  h.port.updateThreadLiveStream(THREAD, () => ({
    threadId: THREAD,
    runId: "run-1",
    assistantEntryId: null,
    pendingAckIntentIds: [],
    streamStatus: "failed",
  }));
  h.liveStreamStateRef.current = h.port.getLiveStreamMap();
  bindings.forceReleaseThreadRuntime(THREAD);
}

test("dual-run: lifecycle matches the legacy run-state/machine orchestration (6b-2a)", async () => {
  lifecycleModule = await import("./transcript-lifecycle.ts");

  const legacyH = makeHarness();
  const legacy = makeLegacyBindings(legacyH);
  replaySequence(legacy, legacyH);

  const nextH = makeHarness();
  const next = makeLifecycleBindings(nextH);
  replaySequence(next, nextH);

  assert.deepEqual(
    nextH.machineTrace,
    legacyH.machineTrace,
    "machine action traces must match",
  );
  assert.deepEqual(
    nextH.liveTrace,
    legacyH.liveTrace,
    "live-stream transition traces must match",
  );
  assert.deepEqual(nextH.getDesktopState(), legacyH.getDesktopState());
  assert.deepEqual(nextH.titleDraftSyncs, legacyH.titleDraftSyncs);
  assert.deepEqual(
    nextH.liveStreamStateRef.current,
    legacyH.liveStreamStateRef.current,
  );
});
