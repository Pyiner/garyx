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
  transcriptControlKind,
  transcriptForCommittedCache,
  transcriptRewriteAction,
  transcriptWithResolvedActiveRun,
} from "../../../shared/transcript-sync.ts";
import { findPendingAckIntentIndex, selectThreadRuntime } from "../message-machine.ts";
import {
  mergeThread,
  teamBlocksEqual,
  threadSummariesEquivalent,
} from "../thread-model.ts";
import { resolveIntentHistoryMatch, visibleTranscriptMessages } from "./transcript-materialize.ts";

function makeHarness() {
  const mirror = new GatewayMirror();
  const machineTrace = [];
  const liveTrace = [];
  const titleDraftSyncs = [];
  const persistTrace = [];
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
    // Slice 2b surface: real mirror commits, recorded persists.
    applyAuthoritativeTranscript: (threadId, transcript) =>
      mirror.applyAuthoritativeTranscript(threadId, transcript),
    applyRemoteTranscript: (threadId, transcript) =>
      mirror.applyRemoteTranscript(threadId, transcript),
    getTranscriptMapsSnapshot: () => mirror.getTranscriptMapsSnapshot(),
    syncThreadUiMessages: (threadId, messages) =>
      mirror.syncThreadUiMessages(threadId, messages),
    persistTranscriptCache(transcript, renderState) {
      persistTrace.push({ transcript, renderState });
    },
  };
  const liveStreamStateRef = { current: {} };
  const refetches = [];
  const deps = {
    setDesktopState: (updater) => {
      desktopState = updater(desktopState);
    },
    syncThreadTitleDraft: (title) => titleDraftSyncs.push(title),
    requestSelectedThreadMessagesBottomSnap: () => {},
    selectedThreadIdRef: { current: "thread::t1" },
    liveStreamStateRef,
    refetchAuthoritativeTranscriptAfterRewrite: async (threadId) => {
      refetches.push(threadId);
    },
  };
  return {
    mirror,
    port,
    deps,
    machineTrace,
    liveTrace,
    titleDraftSyncs,
    persistTrace,
    refetches,
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
  function syncTranscriptRunState(threadId, transcript) {
    return publishTranscriptRunState(
      threadId,
      reduceTranscriptRunState(transcript.messages),
    );
  }
  function applyCommittedTranscriptRunState(event) {
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
  }
  function markIntentsFromHistory(threadId, transcript) {
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
  }
  function applyUserAck(threadId, runId, pendingInputId) {
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
  }
  function forceReleaseThreadRuntime(threadId) {
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
  }

  // ---- 2b apply chain (verbatim pre-slice hook shapes) ----
  function rememberTranscriptSnapshot(
    threadId,
    transcript,
    persist = true,
    syncRunState = true,
  ) {
    if (syncRunState) {
      syncTranscriptRunState(threadId, transcript);
    }
    if (persist) {
      const cacheTranscript = transcriptForCommittedCache(transcript);
      if (
        cacheTranscript.messages.length > 0 ||
        !transcript.threadInfo?.activeRun
      ) {
        h.port.persistTranscriptCache(
          cacheTranscript,
          h.port.getTranscriptMapsSnapshot().renderStateByThread[threadId] ??
            null,
        );
      }
    }
  }
  function applyCanonicalTranscript(threadId, transcript, options) {
    h.port.applyAuthoritativeTranscript(threadId, transcript);
    const resolvedTranscript = transcriptWithResolvedActiveRun(transcript);
    rememberTranscriptSnapshot(
      threadId,
      resolvedTranscript,
      true,
      options?.syncRunState ?? true,
    );
    markIntentsFromHistory(
      threadId,
      visibleTranscriptMessages(resolvedTranscript.messages),
    );
  }
  function threadSummaryFromTranscript(threadId, transcript) {
    if (transcript.thread) {
      return {
        ...transcript.thread,
        agentId:
          transcript.thread.agentId ?? transcript.threadInfo?.agentId ?? null,
        workspacePath:
          transcript.thread.workspacePath ??
          transcript.threadInfo?.workspacePath ??
          null,
        worktree:
          transcript.thread.worktree ?? transcript.threadInfo?.worktree ?? null,
        team: transcript.thread.team ?? transcript.team ?? null,
      };
    }
    const timestamps = transcript.messages
      .map((message) => message.timestamp || "")
      .filter(Boolean);
    const fallbackTimestamp =
      timestamps[timestamps.length - 1] || new Date().toISOString();
    const preview =
      transcript.messages.find((message) => message.text.trim())?.text.trim() ||
      "";
    return {
      id: threadId,
      title: transcript.threadInfo?.agentId || threadId,
      createdAt: timestamps[0] || fallbackTimestamp,
      updatedAt: fallbackTimestamp,
      lastMessagePreview: preview,
      workspacePath: transcript.threadInfo?.workspacePath ?? null,
      messageCount:
        transcript.pageInfo?.totalMessages ?? transcript.messages.length,
      agentId: transcript.threadInfo?.agentId ?? null,
      recentRunId: transcript.threadInfo?.activeRun?.runId ?? null,
      worktree: transcript.threadInfo?.worktree ?? null,
      team: transcript.team ?? null,
    };
  }
  function cacheOpenableTranscriptThread(threadId, transcript) {
    const summary = threadSummaryFromTranscript(threadId, transcript);
    h.deps.setDesktopState((current) => {
      if (!current || current.threads.some((thread) => thread.id === threadId)) {
        return current;
      }
      const existing = current.sessions.find(
        (session) => session.id === threadId,
      );
      if (existing && threadSummariesEquivalent(existing, summary)) {
        return current;
      }
      return {
        ...current,
        sessions: mergeThread(current.sessions, summary),
      };
    });
  }
  function applyRemoteTranscript(threadId, transcript, options) {
    if (!options?.mirrorAlreadyApplied) {
      h.port.applyRemoteTranscript(threadId, transcript);
    }
    const resolvedTranscript = transcriptWithResolvedActiveRun(transcript);
    rememberTranscriptSnapshot(
      threadId,
      resolvedTranscript,
      options?.persist !== false,
      options?.syncRunState ?? true,
    );
    cacheOpenableTranscriptThread(threadId, resolvedTranscript);
    if (resolvedTranscript.team !== undefined) {
      h.deps.setDesktopState((current) => {
        if (!current) {
          return current;
        }
        const nextTeam = resolvedTranscript.team ?? null;
        let changed = false;
        const mapThreadTeam = (thread) => {
          if (thread.id !== threadId) {
            return thread;
          }
          const prev = thread.team ?? null;
          if (teamBlocksEqual(prev, nextTeam)) {
            return thread;
          }
          changed = true;
          return { ...thread, team: nextTeam };
        };
        const nextThreads = current.threads.map(mapThreadTeam);
        const nextSessions = current.sessions.map(mapThreadTeam);
        if (!changed) {
          return current;
        }
        return { ...current, threads: nextThreads, sessions: nextSessions };
      });
    }
    markIntentsFromHistory(
      threadId,
      visibleTranscriptMessages(resolvedTranscript.messages),
    );
  }
  function applyCommittedThreadMessage(event) {
    const threadId = event.threadId;
    if (transcriptRewriteAction(event.message) === "refetch_authoritative") {
      void h.deps.refetchAuthoritativeTranscriptAfterRewrite(threadId);
      return;
    }
    applyCommittedTranscriptRunState(event);
    const merged = h.port.getThreadSnapshotTranscript(threadId);
    if (!merged) {
      return;
    }
    if (h.deps.selectedThreadIdRef.current === threadId) {
      h.deps.requestSelectedThreadMessagesBottomSnap(threadId, true);
    }
    applyRemoteTranscript(threadId, merged, {
      syncRunState: false,
      mirrorAlreadyApplied: true,
    });
    const controlKind = transcriptControlKind(event.message);
    if (controlKind === "user_ack") {
      const control =
        event.message.content &&
        typeof event.message.content === "object" &&
        !Array.isArray(event.message.content)
          ? event.message.content.control
          : null;
      applyUserAck(
        threadId,
        event.runId,
        typeof control?.pending_input_id === "string"
          ? control.pending_input_id
          : typeof control?.pendingInputId === "string"
            ? control.pendingInputId
            : undefined,
      );
    }
  }

  return {
    syncTranscriptRunState,
    applyCommittedTranscriptRunState,
    markIntentsFromHistory,
    applyUserAck,
    forceReleaseThreadRuntime,
    applyCanonicalTranscript,
    applyRemoteTranscript,
    applyCommittedThreadMessage,
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
    applyCanonicalTranscript: (t, tr, o) =>
      lifecycle.acceptAuthoritativeTranscript(t, tr, o),
    applyRemoteTranscript: (t, tr, o) => lifecycle.acceptRemoteTranscript(t, tr, o),
    applyCommittedThreadMessage: (e) => lifecycle.applyCommittedThreadMessage(e),
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

function userMessage(index, text) {
  return {
    id: `${THREAD}:${index}`,
    role: "user",
    kind: "message",
    text,
    timestamp: "2026-07-05T10:00:02Z",
    content: text,
  };
}

function assistantMessage(index, text) {
  return {
    id: `${THREAD}:${index}`,
    role: "assistant",
    kind: "message",
    text,
    timestamp: "2026-07-05T10:00:03Z",
    content: text,
  };
}

function rewriteControlMessage(index) {
  return {
    id: `${THREAD}:${index}`,
    role: "control",
    kind: "control",
    text: "",
    timestamp: "2026-07-05T10:00:04Z",
    content: { control: { kind: "range_rewrite", from_seq: 1, to_seq: 2 } },
  };
}

function replayApplyChainSequence(bindings, h) {
  // 1. Authoritative apply: persist + run-state + intent marking.
  bindings.applyCanonicalTranscript(THREAD, {
    threadId: THREAD,
    messages: [userMessage(0, "hello"), assistantMessage(1, "hi there")],
    pendingInputs: [],
    threadInfo: { agentId: "agent-1", workspacePath: "/Users/test/repo" },
  });
  // 2. Remote apply with a team block on a session-only thread: session
  //    cache write + team propagation into threads/sessions.
  h.deps.setDesktopState((current) => ({
    ...current,
    threads: [],
    sessions: [
      {
        id: THREAD,
        title: "session thread",
        createdAt: "2026-07-05T09:00:00Z",
        updatedAt: "2026-07-05T09:00:00Z",
      },
    ],
  }));
  bindings.applyRemoteTranscript(
    THREAD,
    {
      threadId: THREAD,
      messages: [
        userMessage(0, "hello"),
        assistantMessage(1, "hi there"),
        assistantMessage(2, "follow-up"),
      ],
      pendingInputs: [],
      threadInfo: { agentId: "agent-1", workspacePath: "/Users/test/repo" },
      team: { teamId: "team-1", name: "Test Team" },
    },
    { persist: true },
  );
  // 3. Committed side-effect step over the folded snapshot.
  bindings.applyCommittedThreadMessage({
    type: "committed_message",
    threadId: THREAD,
    runId: "run-2",
    seq: 12,
    message: assistantMessage(3, "committed reply"),
  });
  // 4. Committed rewrite control routes to the (transitional) refetch seam.
  bindings.applyCommittedThreadMessage({
    type: "committed_message",
    threadId: THREAD,
    runId: "run-2",
    seq: 13,
    message: rewriteControlMessage(4),
  });
}

test("dual-run: lifecycle matches the legacy apply chain (6b-2b)", async () => {
  lifecycleModule = await import("./transcript-lifecycle.ts");

  const legacyH = makeHarness();
  const legacy = makeLegacyBindings(legacyH);
  replayApplyChainSequence(legacy, legacyH);

  const nextH = makeHarness();
  const next = makeLifecycleBindings(nextH);
  replayApplyChainSequence(next, nextH);

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
  assert.deepEqual(
    nextH.persistTrace,
    legacyH.persistTrace,
    "persist ride-along traces must match",
  );
  assert.deepEqual(nextH.refetches, legacyH.refetches);
  assert.deepEqual(nextH.getDesktopState(), legacyH.getDesktopState());
  assert.deepEqual(nextH.titleDraftSyncs, legacyH.titleDraftSyncs);
  assert.deepEqual(
    nextH.liveStreamStateRef.current,
    legacyH.liveStreamStateRef.current,
  );
  assert.ok(
    nextH.persistTrace.length > 0,
    "the sequence must exercise the persist ride-along",
  );
  assert.equal(nextH.refetches.length, 1, "the rewrite must hit the refetch seam");
  const sessions = nextH.getDesktopState().sessions;
  assert.equal(
    sessions.find((s) => s.id === THREAD)?.team?.teamId,
    "team-1",
    "the team block must propagate into the session cache",
  );
});

// #TASK-1633 regression: lifecycle dispatches run inside the mirror and
// bypass the old AppShell warming proxy (`messageStateRef.current =
// dispatch(...)`). The AppShell-shaped delegate is now a stable getter
// over the mirror's machine state; event-path readers must observe a
// lifecycle dispatch synchronously — a plain ref shadow would still hold
// the pre-dispatch state here.
test("messageStateRef getter stays warm across lifecycle dispatches (TASK-1633)", async () => {
  lifecycleModule = await import("./transcript-lifecycle.ts");
  const { GatewayMirror: Mirror } = await import("./mirror.ts");
  const { TranscriptLifecycle } = require_lifecycle();

  const mirror = new Mirror();
  // The real AppShell delegate shape (AppShell.tsx messageStateRef).
  const messageStateRef = {
    get current() {
      return mirror.getMachineState();
    },
  };
  const lifecycle = new TranscriptLifecycle(mirror);
  lifecycle.setDeps({
    setDesktopState: () => {},
    syncThreadTitleDraft: () => {},
    requestSelectedThreadMessagesBottomSnap: () => {},
    selectedThreadIdRef: { current: THREAD },
    liveStreamStateRef: { current: {} },
    refetchAuthoritativeTranscriptAfterRewrite: async () => {},
  });

  lifecycle.setThreadRuntimeState(THREAD, "running_remote", {
    remoteRunId: "run-9",
  });
  const runtime = messageStateRef.current.threadRuntimeByThread[THREAD];
  assert.ok(runtime, "runtime entry must be visible through the getter");
  assert.equal(runtime.state, "running_remote");
  assert.equal(runtime.remoteRunId, "run-9");
});
