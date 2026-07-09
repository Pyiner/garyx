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
  isThreadStreamGapError,
  mergeForwardTranscriptPage,
  reduceTranscriptRunState,
  shouldRestartSelectedThreadStreamAfterRefetch,
  streamResumeCursor,
  transcriptCommittedAfterCursor,
  transcriptControlKind,
  transcriptForCommittedCache,
  transcriptRewriteAction,
  transcriptWithResolvedActiveRun,
} from "../../../shared/transcript-sync.ts";
import { isTransientGatewayErrorMessage } from "../app-shell/gateway-errors.ts";
import {
  chatStreamEventHasRunLifecycle,
  isMissingThreadTranscript,
  reconcileAssistantEntriesForGatewayRecovery,
  transcriptHasAutomationResponse,
} from "./transcript-materialize.ts";
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
  const ipcTrace = [];
  const cachedTranscriptByThread = {};
  const remoteTranscriptByThread = {};
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
    // Slice 2c surface: recorded IPC stubs (fetch/stream/cache).
    ingest: (event) => mirror.ingest(event),
    clearThreadTranscript: (threadId) => mirror.clearThreadTranscript(threadId),
    fetchOlderThreadHistoryPage: (threadId, options) =>
      mirror.fetchOlderThreadHistoryPage(threadId, options),
    startThreadStream(input) {
      ipcTrace.push(["startThreadStream", input]);
      return Promise.resolve();
    },
    stopThreadStream(input) {
      ipcTrace.push(["stopThreadStream", input]);
      return Promise.resolve();
    },
    loadThreadTranscriptCache(threadId) {
      ipcTrace.push(["loadThreadTranscriptCache", threadId]);
      return Promise.resolve(cachedTranscriptByThread[threadId] ?? null);
    },
    clearThreadTranscriptCache(threadId) {
      ipcTrace.push(["clearThreadTranscriptCache", threadId]);
      return Promise.resolve();
    },
    getThreadHistoryFull(threadId) {
      ipcTrace.push(["getThreadHistoryFull", threadId]);
      return Promise.resolve(
        remoteTranscriptByThread[threadId] ?? {
          threadId,
          messages: [],
          pendingInputs: [],
          threadInfo: null,
        },
      );
    },
    getThreadHistoryPage(input) {
      ipcTrace.push(["getThreadHistoryPage", input]);
      return Promise.resolve({
        threadId: input.threadId,
        messages: [],
        pendingInputs: [],
        threadInfo: null,
        pageInfo: { hasMoreAfter: false },
      });
    },
  };
  const liveStreamStateRef = { current: {} };
  const seamTrace = [];
  const deps = {
    setDesktopState: (updater) => {
      desktopState = updater(desktopState);
    },
    syncThreadTitleDraft: (title) => titleDraftSyncs.push(title),
    requestSelectedThreadMessagesBottomSnap: () => {},
    selectedThreadIdRef: { current: "thread::t1" },
    liveStreamStateRef,
    setError: (error) => seamTrace.push(["setError", error]),
    setHistoryLoading: (loading) => seamTrace.push(["setHistoryLoading", loading]),
    setPendingAutomationRun: (threadId, run) =>
      seamTrace.push(["setPendingAutomationRun", threadId, run]),
    recordGatewayStatusObservation: (status, reason) =>
      seamTrace.push(["recordGatewayStatusObservation", status, reason]),
    scheduleDesktopStateRefresh: () =>
      seamTrace.push(["scheduleDesktopStateRefresh"]),
    scheduleHistoryRefresh: (threadId, attempts, delayMs, canonical) =>
      seamTrace.push([
        "scheduleHistoryRefresh",
        threadId,
        attempts,
        delayMs,
        canonical,
      ]),
    connection: { ok: true, bridgeReady: true, gatewayUrl: "http://gw.test" },
    settingsDraft: { gatewayUrl: "http://draft.test" },
    selectedThreadGenerationRef: { current: 1 },
    lastRenderedMessageThreadRef: { current: null },
    messagesRef: { current: null },
    pendingMessagesPrependAnchorRef: { current: null },
    sideChatThreadIdRef: { current: null },
    sideChatStreamConsumerId: (threadId) => `side-chat:${threadId}`,
  };
  return {
    mirror,
    port,
    deps,
    machineTrace,
    liveTrace,
    titleDraftSyncs,
    persistTrace,
    ipcTrace,
    seamTrace,
    cachedTranscriptByThread,
    remoteTranscriptByThread,
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
    return h.port.getLiveStreamMap()[threadId] || null;
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
      void refetchAuthoritativeTranscriptAfterRewrite(threadId);
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

  // ---- 2c fetch/stream lifecycle (verbatim pre-slice hook + AppShell) ----
  function startCommittedThreadStream(threadId, transcript, consumerId) {
    const renderFloor =
      h.port.getTranscriptMapsSnapshot().renderStateByThread[threadId]?.window
        ?.floor_seq ?? 0;
    return h.port.startThreadStream({
      threadId,
      consumerId,
      afterSeq: streamResumeCursor({
        afterCursor: transcriptCommittedAfterCursor(transcript),
        fallbackMaxIndex: null,
      }),
      ...(renderFloor > 0 ? { renderFloor } : {}),
    });
  }
  async function refetchAuthoritativeTranscriptAfterRewrite(threadId) {
    const startSelectionGeneration = h.deps.selectedThreadGenerationRef.current;
    try {
      await h.port.clearThreadTranscriptCache(threadId);
      const transcript = await h.port.getThreadHistoryFull(threadId);
      if (h.deps.selectedThreadIdRef.current === threadId) {
        h.deps.requestSelectedThreadMessagesBottomSnap(threadId, true);
      }
      applyRemoteTranscript(threadId, transcript);
      const shouldRestartSelectedStream =
        shouldRestartSelectedThreadStreamAfterRefetch({
          threadId,
          selectedThreadId: h.deps.selectedThreadIdRef.current,
          startSelectionGeneration,
          currentSelectionGeneration: h.deps.selectedThreadGenerationRef.current,
        });
      if (shouldRestartSelectedStream) {
        await startCommittedThreadStream(threadId, transcript, "selected-thread");
      }
      if (h.deps.sideChatThreadIdRef.current === threadId) {
        await startCommittedThreadStream(
          threadId,
          transcript,
          h.deps.sideChatStreamConsumerId(threadId),
        );
      }
    } catch {
      h.deps.scheduleHistoryRefresh(threadId, 3, 500, true);
    }
  }
  async function fetchSelectedThreadIncrementalTranscript(
    threadId,
    cached,
    isCancelled,
  ) {
    let current = cached;
    let cursor = transcriptCommittedAfterCursor(current);
    if (!current || cursor === null) {
      return {
        transcript: await h.port.getThreadHistoryFull(threadId),
        authoritative: true,
      };
    }
    // (paging branch elided: the replay fixtures always start cache-less or
    // with a cursor-less cache, so the legacy loop is unreachable here.)
    return { transcript: current, authoritative: false };
  }
  async function loadSelectedThreadTranscriptFromSingleSource(
    threadId,
    isCancelled,
  ) {
    const hasRenderedThread =
      h.deps.lastRenderedMessageThreadRef.current === threadId;
    const hasCachedMessages =
      (h.port.getTranscriptMapsSnapshot().messagesByThread[threadId] || [])
        .length > 0;
    h.deps.requestSelectedThreadMessagesBottomSnap(
      threadId,
      !hasRenderedThread || !hasCachedMessages,
    );
    h.deps.setHistoryLoading(true);
    h.deps.setError(null);
    let latestTranscript = h.port.getThreadSnapshotTranscript(threadId);
    let streamReady = false;
    let streamStarted = false;
    try {
      const cached = await h.port.loadThreadTranscriptCache(threadId);
      if (isCancelled()) {
        return;
      }
      if (cached) {
        latestTranscript = cached.transcript;
        applyRemoteTranscript(threadId, cached.transcript, { persist: false });
        if (cached.renderState) {
          h.port.ingest({
            type: "thread_render_frame",
            threadId,
            events: [],
            renderState: cached.renderState,
          });
        }
        await startCommittedThreadStream(
          threadId,
          cached.transcript,
          "selected-thread",
        );
        streamStarted = true;
      }
      const fetched = await fetchSelectedThreadIncrementalTranscript(
        threadId,
        latestTranscript,
        isCancelled,
      );
      if (isCancelled()) {
        return;
      }
      if (
        fetched.authoritative &&
        isMissingThreadTranscript(fetched.transcript)
      ) {
        if (streamStarted) {
          await h.port.stopThreadStream({
            threadId,
            consumerId: "selected-thread",
          });
          streamStarted = false;
        }
        if (latestTranscript) {
          void h.port.clearThreadTranscriptCache(threadId);
          h.port.clearThreadTranscript(threadId);
          latestTranscript = null;
        }
        h.deps.setError(`Thread not found: ${threadId}`);
        return;
      }
      h.deps.requestSelectedThreadMessagesBottomSnap(threadId, true);
      latestTranscript = fetched.authoritative
        ? fetched.transcript
        : mergeForwardTranscriptPage(
            h.port.getThreadSnapshotTranscript(threadId),
            fetched.transcript,
          );
      applyRemoteTranscript(threadId, latestTranscript);
      if (transcriptHasAutomationResponse(latestTranscript.messages)) {
        h.deps.setPendingAutomationRun(threadId, null);
      }
      streamReady = true;
    } catch (historyError) {
      if (!latestTranscript) {
        h.deps.setError(
          historyError instanceof Error
            ? historyError.message
            : "Failed to load thread history",
        );
      } else {
        h.deps.setError(
          historyError instanceof Error
            ? `Failed to sync latest thread history: ${historyError.message}`
            : "Failed to sync latest thread history",
        );
      }
    } finally {
      if (!isCancelled()) {
        h.deps.setHistoryLoading(false);
        if (!(streamStarted || !streamReady || !latestTranscript)) {
          await startCommittedThreadStream(
            threadId,
            latestTranscript,
            "selected-thread",
          );
        }
      }
    }
  }
  function handleChatStreamEvent(event) {
    const threadId = event.threadId;
    if (event.type === "thread_render_frame") {
      for (const committed of event.events) {
        applyCommittedThreadMessage(committed);
      }
      return;
    }
    if (event.type !== "error") {
      return;
    }
    const currentStream = getLiveStreamState(threadId);
    const activeIntentId = currentStream?.activeIntentId;
    if (isThreadStreamGapError(event)) {
      if (activeIntentId) {
        h.port.dispatchMachineAction({
          type: "intent/awaiting-history",
          intentId: activeIntentId,
        });
      }
      updateLiveStreamState(threadId, (current) =>
        current
          ? {
              ...current,
              runId: event.runId,
              assistantEntryId: null,
              streamStatus: "reconciling",
            }
          : null,
      );
      setThreadRuntimeState(threadId, "reconciling_history", {
        activeIntentId: activeIntentId || undefined,
        remoteRunId: event.runId,
      });
      void refetchAuthoritativeTranscriptAfterRewrite(threadId);
      return;
    }
    const recoveryResult = activeIntentId
      ? reconcileAssistantEntriesForGatewayRecovery(
          h.port.getTranscriptMapsSnapshot().messagesByThread[threadId] || [],
          activeIntentId,
          [currentStream?.assistantEntryId],
        )
      : { entries: [], matched: false };
    const isTerminalRunError = event.terminal === true;
    if (
      !isTerminalRunError &&
      (isTransientGatewayErrorMessage(event.error) || recoveryResult.matched)
    ) {
      const recoveryStatusLabel = "Waiting to sync with gateway…";
      h.deps.recordGatewayStatusObservation(
        {
          ok: false,
          bridgeReady: false,
          gatewayUrl:
            h.deps.connection?.gatewayUrl || h.deps.settingsDraft.gatewayUrl,
          error: event.error,
        },
        recoveryStatusLabel,
      );
      let assistantEntryId = null;
      updateLiveStreamState(threadId, (current) => {
        assistantEntryId = current?.assistantEntryId ?? null;
        return current
          ? {
              ...current,
              runId: event.runId,
              assistantEntryId: null,
              streamStatus: "disconnected",
            }
          : null;
      });
      if (activeIntentId) {
        h.port.dispatchMachineAction({
          type: "intent/awaiting-history",
          intentId: activeIntentId,
        });
      }
      setThreadRuntimeState(threadId, "reconciling_history", {
        activeIntentId: activeIntentId || undefined,
        remoteRunId: event.runId,
      });
      if (activeIntentId) {
        updateMessagesByThread((current) => {
          const nextEntries = reconcileAssistantEntriesForGatewayRecovery(
            current[threadId] || [],
            activeIntentId,
            [assistantEntryId],
          ).entries;
          return {
            ...current,
            [threadId]: nextEntries,
          };
        });
      }
      h.deps.scheduleHistoryRefresh(threadId, 5, 1200, true);
      return;
    }
    updateLiveStreamState(threadId, (current) =>
      current
        ? {
            ...current,
            runId: event.runId,
            assistantEntryId: null,
            streamStatus: "failed",
          }
        : null,
    );
    if (activeIntentId) {
      h.port.dispatchMachineAction({
        type: "intent/failed",
        intentId: activeIntentId,
        error: event.error,
      });
    }
    setThreadRuntimeState(threadId, "failed", {
      activeIntentId: activeIntentId || undefined,
      remoteRunId: event.runId,
      error: event.error,
    });
    h.deps.setError(event.error);
  }
  function updateMessagesByThread(updater) {
    const previous = h.port.getTranscriptMapsSnapshot().messagesByThread;
    const next = updater(previous);
    if (next !== previous) {
      for (const threadId of Object.keys(next)) {
        if (next[threadId] !== previous[threadId]) {
          h.port.syncThreadUiMessages(threadId, next[threadId]);
        }
      }
      for (const threadId of Object.keys(previous)) {
        if (!(threadId in next)) {
          h.port.syncThreadUiMessages(threadId, []);
        }
      }
    }
    return next;
  }
  function notifyStreamEvent(event) {
    h.port.ingest(event);
    if (chatStreamEventHasRunLifecycle(event)) {
      h.deps.scheduleDesktopStateRefresh();
    }
    handleChatStreamEvent(event);
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
    startCommittedThreadStream,
    refetchAuthoritativeTranscriptAfterRewrite,
    loadSelectedThreadTranscript: (threadId) =>
      loadSelectedThreadTranscriptFromSingleSource(threadId, () => false),
    notifyStreamEvent,
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
    startCommittedThreadStream: (t, tr, c) =>
      lifecycle.startCommittedThreadStream(t, tr, c),
    refetchAuthoritativeTranscriptAfterRewrite: (t) =>
      lifecycle.refetchAuthoritativeTranscriptAfterRewrite(t),
    loadSelectedThreadTranscript: (t) => lifecycle.loadSelectedThreadTranscript(t),
    notifyStreamEvent: (e) => lifecycle.notifyStreamEvent(e),
    cancelSelectedThreadLoad: (t) => lifecycle.cancelSelectedThreadLoad(t),
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
    nextH.port.getLiveStreamMap(),
    legacyH.port.getLiveStreamMap(),
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
  // The rewrite step (4) runs the real single-owner refetch; give its
  // authoritative fetch a deterministic transcript (timestamps included)
  // so no code path falls back to wall-clock summaries.
  h.remoteTranscriptByThread[THREAD] = {
    threadId: THREAD,
    messages: [userMessage(0, "hello"), assistantMessage(1, "hi there")],
    pendingInputs: [],
    threadInfo: { agentId: "agent-1" },
    team: { teamId: "team-1", name: "Test Team" },
  };
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

async function flushAsync() {
  for (let i = 0; i < 8; i += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
}

test("dual-run: lifecycle matches the legacy apply chain (6b-2b)", async () => {
  lifecycleModule = await import("./transcript-lifecycle.ts");

  const legacyH = makeHarness();
  const legacy = makeLegacyBindings(legacyH);
  replayApplyChainSequence(legacy, legacyH);
  await flushAsync();

  const nextH = makeHarness();
  const next = makeLifecycleBindings(nextH);
  replayApplyChainSequence(next, nextH);
  await flushAsync();

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
  assert.deepEqual(
    nextH.ipcTrace,
    legacyH.ipcTrace,
    "fetch/stream IPC traces must match",
  );
  assert.deepEqual(nextH.getDesktopState(), legacyH.getDesktopState());
  assert.deepEqual(nextH.titleDraftSyncs, legacyH.titleDraftSyncs);
  assert.deepEqual(
    nextH.port.getLiveStreamMap(),
    legacyH.port.getLiveStreamMap(),
  );
  assert.ok(
    nextH.persistTrace.length > 0,
    "the sequence must exercise the persist ride-along",
  );
  assert.ok(
    nextH.ipcTrace.some(([name]) => name === "getThreadHistoryFull"),
    "the rewrite must run the single-owner refetch",
  );
  const sessions = nextH.getDesktopState().sessions;
  assert.equal(
    sessions.find((s) => s.id === THREAD)?.team?.teamId,
    "team-1",
    "the team block must propagate into the session cache",
  );
});

async function replayLifecycleSequence(bindings, h) {
  // 1. Cold selected-thread load: no disk cache, authoritative full fetch,
  //    then the committed stream starts from the fetched cursor.
  h.remoteTranscriptByThread[THREAD] = {
    threadId: THREAD,
    messages: [userMessage(0, "hello"), assistantMessage(1, "hi there")],
    pendingInputs: [],
    threadInfo: { agentId: "agent-1" },
  };
  await bindings.loadSelectedThreadTranscript(THREAD);
  // 2. Committed frame through the stream-listener entry.
  bindings.notifyStreamEvent({
    type: "thread_render_frame",
    threadId: THREAD,
    events: [
      {
        type: "committed_message",
        threadId: THREAD,
        runId: "run-5",
        seq: 21,
        message: assistantMessage(2, "streamed reply"),
      },
    ],
    renderState: null,
  });
  // 3. Terminal stream error (failed branch: machine + live-stream + error).
  h.port.updateThreadLiveStream(THREAD, () => ({
    threadId: THREAD,
    runId: "run-5",
    activeIntentId: undefined,
    assistantEntryId: null,
    pendingAckIntentIds: [],
    streamStatus: "streaming",
  }));
  h.liveStreamStateRef.current = h.port.getLiveStreamMap();
  bindings.notifyStreamEvent({
    type: "error",
    threadId: THREAD,
    runId: "run-5",
    error: "provider exploded",
    terminal: true,
  });
  await flushAsync();
}

test("dual-run: lifecycle matches the legacy fetch/stream lifecycle (6b-2c)", async () => {
  lifecycleModule = await import("./transcript-lifecycle.ts");

  const legacyH = makeHarness();
  const legacy = makeLegacyBindings(legacyH);
  await replayLifecycleSequence(legacy, legacyH);

  const nextH = makeHarness();
  const next = makeLifecycleBindings(nextH);
  await replayLifecycleSequence(next, nextH);

  assert.deepEqual(nextH.ipcTrace, legacyH.ipcTrace, "IPC traces must match");
  assert.deepEqual(
    nextH.seamTrace,
    legacyH.seamTrace,
    "React-seam traces must match",
  );
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
  assert.deepEqual(
    nextH.port.getLiveStreamMap(),
    legacyH.port.getLiveStreamMap(),
  );
  // The sequence must actually exercise the load + stream + failure chain.
  assert.deepEqual(
    nextH.ipcTrace.map(([name]) => name).slice(0, 3),
    ["loadThreadTranscriptCache", "getThreadHistoryFull", "startThreadStream"],
  );
  assert.ok(
    nextH.seamTrace.some(
      ([name, value]) => name === "setError" && value === "provider exploded",
    ),
    "the terminal error must surface through setError",
  );
});

test("2c: cache-restored windowed snapshot pins render_floor on the committed stream (#TASK-1715)", async () => {
  lifecycleModule = await import("./transcript-lifecycle.ts");
  const h = makeHarness();
  const bindings = makeLifecycleBindings(h);
  const transcript = {
    threadId: THREAD,
    messages: [userMessage(0, "hello")],
    pendingInputs: [],
    threadInfo: { agentId: "agent-1" },
  };
  h.cachedTranscriptByThread[THREAD] = {
    transcript,
    renderState: {
      based_on_seq: 42,
      rows: [],
      tailActivity: "none",
      activeToolGroupId: null,
      progress_locus: "none",
      filtered_placeholders: [],
      window: { floor_seq: 37, has_more_above: true },
    },
  };
  h.remoteTranscriptByThread[THREAD] = transcript;

  await bindings.loadSelectedThreadTranscript(THREAD);
  await flushAsync();

  const start = h.ipcTrace.find(([name]) => name === "startThreadStream");
  assert.ok(start, "cache-restored reopen must start the committed stream");
  assert.equal(start[1].threadId, THREAD);
  assert.equal(
    start[1].renderFloor,
    37,
    "the restored window floor must ride the stream start IPC",
  );
});

test("2c: stream start without a render window sends no renderFloor (#TASK-1715)", async () => {
  lifecycleModule = await import("./transcript-lifecycle.ts");
  const h = makeHarness();
  const bindings = makeLifecycleBindings(h);
  h.remoteTranscriptByThread[THREAD] = {
    threadId: THREAD,
    messages: [userMessage(0, "hello")],
    pendingInputs: [],
    threadInfo: { agentId: "agent-1" },
  };

  await bindings.loadSelectedThreadTranscript(THREAD);
  await flushAsync();

  const start = h.ipcTrace.find(([name]) => name === "startThreadStream");
  assert.ok(start, "load must start the committed stream");
  assert.ok(
    !("renderFloor" in start[1]),
    "no window floor -> the stream start input stays byte-identical to today",
  );
});

test("2c: cancelSelectedThreadLoad invalidates a superseded load and stops the stream", async () => {
  lifecycleModule = await import("./transcript-lifecycle.ts");
  const h = makeHarness();
  const bindings = makeLifecycleBindings(h);
  h.remoteTranscriptByThread[THREAD] = {
    threadId: THREAD,
    messages: [userMessage(0, "hello")],
    pendingInputs: [],
    threadInfo: { agentId: "agent-1" },
  };
  const load = bindings.loadSelectedThreadTranscript(THREAD);
  // Cancel synchronously: the fetch settles afterwards and must not land.
  bindings.cancelSelectedThreadLoad(THREAD);
  await load;
  await flushAsync();
  assert.ok(
    h.ipcTrace.some(([name]) => name === "stopThreadStream"),
    "cancel must stop the selected-thread stream consumer",
  );
  assert.ok(
    !h.ipcTrace.some(([name]) => name === "startThreadStream"),
    "a cancelled load must not start the committed stream",
  );
  assert.ok(
    !h.seamTrace.some(
      ([name, value]) => name === "setHistoryLoading" && value === false,
    ),
    "a cancelled load must not write post-await loading state",
  );
});

test("2c: concurrent rewrite refetch triggers coalesce into one fetch", async () => {
  lifecycleModule = await import("./transcript-lifecycle.ts");
  const h = makeHarness();
  const bindings = makeLifecycleBindings(h);
  h.remoteTranscriptByThread[THREAD] = {
    threadId: THREAD,
    messages: [userMessage(0, "hello")],
    pendingInputs: [],
    threadInfo: { agentId: "agent-1" },
  };
  const first = bindings.refetchAuthoritativeTranscriptAfterRewrite(THREAD);
  const second = bindings.refetchAuthoritativeTranscriptAfterRewrite(THREAD);
  await Promise.all([first, second]);
  await flushAsync();
  assert.equal(
    h.ipcTrace.filter(([name]) => name === "getThreadHistoryFull").length,
    1,
    "concurrent triggers must coalesce into one authoritative fetch",
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
