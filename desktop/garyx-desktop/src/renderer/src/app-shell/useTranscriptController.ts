import { startTransition, useEffect, useRef } from "react";

import type {
  ConnectionStatus,
  DesktopChatStreamEvent,
  DesktopSettings,
  DesktopState,
  DesktopThreadSummary,
  PendingThreadInput,
  RenderState,
  ThreadRuntimeInfo,
  ThreadTranscript,
  TranscriptMessage,
} from "@shared/contracts";
import {
  applyTranscriptRunStateRecord,
  decideTranscriptFetchPageAction,
  isControlTranscriptMessage,
  isThreadStreamGapError,
  mergeForwardTranscriptPage,
  reduceTranscriptRunState,
  shouldRefetchAuthoritativeAfterForwardPageLimit,
  streamResumeCursor,
  transcriptCommittedAfterCursor,
  transcriptControlKind,
  transcriptForCommittedCache,
  transcriptRewriteAction,
  transcriptWithResolvedActiveRun,
  type TranscriptRunState,
} from "@shared/transcript-sync";

import {
  findPendingAckIntentIndex,
  selectThreadRuntime,
  type MessageIntent,
  type MessageMachineAction,
  type MessageMachineState,
  type ThreadRuntimeState,
} from "../message-machine";
import {
  countTranscriptFiles,
  countTranscriptImages,
  extractTranscriptText,
} from "../message-rich-content";
import {
  mergeThread,
  teamBlocksEqual,
  threadSummariesEquivalent,
} from "../thread-model";
import { isTransientGatewayErrorMessage } from "./gateway-errors";
import { extractImageGenerationImageContent } from "./image-generation-content";
import { isRunLoadingPlaceholderMessage } from "./loading-labels";
import type {
  LiveStreamState,
  MessageMap,
  PendingAutomationRun,
  PendingThreadInputMap,
  UiTranscriptMessage,
} from "./types";

const THREAD_HISTORY_FORWARD_PAGE_LIMIT = 50;
export const SELECTED_THREAD_STREAM_CONSUMER_ID = "selected-thread";

// Batch 2a-1/2a-2: the pure materialization + remote-apply helpers live in
// gateway-mirror; re-export the public ones and import the internals the
// hook still uses.
export {
  messagesNearEarlierUserTurnBoundary,
  normalizeMessageText,
  reconcileAssistantEntriesForGatewayRecovery,
  resolveIntentHistoryMatch,
  transcriptHasAutomationResponse,
  transcriptMessageMatchesIntent,
  userMessageIdForOrigin,
  type ThreadHistoryPaginationState,
} from "../gateway-mirror/transcript-materialize";
import {
  chatStreamEventHasRunLifecycle,
  committedMessageForwardPage,
  messagesNearEarlierUserTurnBoundary,
  mergeRemotePaginationState,
  mergeRemoteTranscriptWithLocal,
  paginationStateFromTranscript,
  transcriptHasAutomationResponse,
  reconcileAssistantEntriesForGatewayRecovery,
  resolveIntentHistoryMatch,
  userMessageIdForOrigin,
  materializeRemoteTranscript,
  visibleTranscriptMessages,
  THREAD_HISTORY_PAGE_SIZE,
  THREAD_HISTORY_USER_QUERY_LIMIT,
  type ThreadHistoryPaginationState,
} from "../gateway-mirror/transcript-materialize";
import type { GatewayMirror } from "../gateway-mirror/mirror";

type UseTranscriptControllerArgs = {
  activeHistoryPagination: ThreadHistoryPaginationState | null;
  activeMessages: UiTranscriptMessage[];
  activeThreadMessageKey: string | null;
  connection: ConnectionStatus | null;
  desktopState: DesktopState | null;
  dispatchMessageState: (action: MessageMachineAction) => void;
  editingThreadTitle: boolean;
  historyLoading: boolean;
  lastRenderedMessageThreadRef: React.MutableRefObject<string | null>;
  liveStreamStateRef: React.MutableRefObject<Record<string, LiveStreamState>>;
  messageStateRef: React.MutableRefObject<MessageMachineState>;
  messagesRef: React.MutableRefObject<HTMLDivElement | null>;
  mirror: GatewayMirror;
  pendingMessagesPrependAnchorRef: React.MutableRefObject<{
    threadId: string;
    scrollHeight: number;
    scrollTop: number;
  } | null>;
  recordGatewayStatusObservation: (
    status: ConnectionStatus | null,
    reason?: string | null,
  ) => void;
  refetchAuthoritativeTranscriptAfterRewrite: (
    threadId: string,
  ) => Promise<void>;
  requestSelectedThreadMessagesBottomSnap: (
    threadId: string | null | undefined,
    forceStick?: boolean,
  ) => void;
  scheduleDesktopStateRefresh: (delayMs?: number) => void;
  scheduleHistoryRefresh: (
    threadId: string,
    attempts?: number,
    delayMs?: number,
    canonical?: boolean,
  ) => void;
  selectedThreadId: string | null;
  selectedThreadIdRef: React.MutableRefObject<string | null>;
  setDesktopState: React.Dispatch<React.SetStateAction<DesktopState | null>>;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  setHistoryLoading: React.Dispatch<React.SetStateAction<boolean>>;
  setHistoryPaginationByThread: React.Dispatch<
    React.SetStateAction<Record<string, ThreadHistoryPaginationState>>
  >;
  setLiveStreamStateByThread: React.Dispatch<
    React.SetStateAction<Record<string, LiveStreamState>>
  >;
  setMessagesByThread: React.Dispatch<React.SetStateAction<MessageMap>>;
  setPendingAutomationRun: (
    threadId: string,
    run: PendingAutomationRun | null,
  ) => void;
  setPendingRemoteInputsByThread: React.Dispatch<
    React.SetStateAction<PendingThreadInputMap>
  >;
  setRenderStateByThread: React.Dispatch<
    React.SetStateAction<Record<string, RenderState>>
  >;
  setThreadInfoByThread: React.Dispatch<
    React.SetStateAction<Record<string, ThreadRuntimeInfo | null>>
  >;
  setTitleDraft: React.Dispatch<React.SetStateAction<string>>;
  settingsDraft: DesktopSettings;
};

export function useTranscriptController({
  activeHistoryPagination,
  activeMessages,
  activeThreadMessageKey,
  connection,
  desktopState,
  dispatchMessageState,
  editingThreadTitle,
  historyLoading,
  lastRenderedMessageThreadRef,
  liveStreamStateRef,
  messageStateRef,
  messagesRef,
  mirror,
  pendingMessagesPrependAnchorRef,
  recordGatewayStatusObservation,
  refetchAuthoritativeTranscriptAfterRewrite,
  requestSelectedThreadMessagesBottomSnap,
  scheduleDesktopStateRefresh,
  scheduleHistoryRefresh,
  selectedThreadId,
  selectedThreadIdRef,
  setDesktopState,
  setError,
  setHistoryLoading,
  setHistoryPaginationByThread,
  setLiveStreamStateByThread,
  setMessagesByThread,
  setPendingAutomationRun,
  setPendingRemoteInputsByThread,
  setRenderStateByThread,
  setThreadInfoByThread,
  setTitleDraft,
  settingsDraft,
}: UseTranscriptControllerArgs) {
  const messagesByThreadRef = useRef<MessageMap>({});
  const renderStateByThreadRef = useRef<Record<string, RenderState>>({});
  const transcriptSnapshotByThreadRef = useRef<Record<string, ThreadTranscript>>(
    {},
  );
  const transcriptRunStateByThreadRef = useRef<Record<string, TranscriptRunState>>(
    {},
  );
  const historyPaginationByThreadRef = useRef<
    Record<string, ThreadHistoryPaginationState>
  >({});
  const threadTitleOverridesRef = useRef<Record<string, string>>({});
  const streamEventHandlerRef = useRef<(event: DesktopChatStreamEvent) => void>(
    () => {},
  );

  useEffect(() => {
    streamEventHandlerRef.current = handleChatStreamEvent;
  });

  /**
   * Batch 2b dual-write scaffolding (deleted with the legacy path in batch
   * 6): every transcript input is fed to the GatewayMirror alongside the
   * legacy React state so the two stay converged while the legacy path
   * still renders. Mirror convergence must never break the legacy render
   * path — divergence is surfaced by the dev parity probe, not by throwing.
   */
  function mirrorDualWrite(operation: () => void) {
    try {
      operation();
    } catch (mirrorError) {
      console.error("[gateway-mirror] dual-write failed", mirrorError);
    }
  }

  useEffect(() => {
    const listener = (event: DesktopChatStreamEvent) => {
      // Batch 2b dual-feed: the mirror is a first-class consumer of the
      // chat stream (frames commit atomically inside ingest). The legacy
      // handler keeps owning machine/live-stream/error side effects.
      mirrorDualWrite(() => mirror.ingest(event));
      if (chatStreamEventHasRunLifecycle(event)) {
        scheduleDesktopStateRefresh();
      }
      streamEventHandlerRef.current(event);
    };
    window.garyxDesktop.subscribeChatStream(listener);
    return () => {
      window.garyxDesktop.unsubscribeChatStream(listener);
    };
  }, []);

  useEffect(() => {
    if (!selectedThreadId || !desktopState) {
      return;
    }

    let cancelled = false;
    void loadSelectedThreadTranscriptFromSingleSource(
      selectedThreadId,
      () => cancelled,
    );

    return () => {
      cancelled = true;
      void window.garyxDesktop.stopThreadStream({
        threadId: selectedThreadId,
        consumerId: SELECTED_THREAD_STREAM_CONSUMER_ID,
      });
    };
  }, [Boolean(desktopState), selectedThreadId]);

  useEffect(() => {
    if (
      !activeThreadMessageKey ||
      historyLoading ||
      !activeHistoryPagination?.hasMoreBefore ||
      activeHistoryPagination.loadingBefore
    ) {
      return;
    }

    const node = messagesRef.current;
    if (!messagesNearEarlierUserTurnBoundary(node)) {
      return;
    }

    const threadId = activeThreadMessageKey;
    const timer = window.setTimeout(() => {
      if (selectedThreadIdRef.current === threadId) {
        void loadOlderThreadHistoryPage(threadId);
      }
    }, 0);

    return () => {
      window.clearTimeout(timer);
    };
  }, [
    activeThreadMessageKey,
    activeMessages.length,
    activeHistoryPagination?.hasMoreBefore,
    activeHistoryPagination?.loadingBefore,
    activeHistoryPagination?.nextBeforeIndex,
    historyLoading,
  ]);

  function intentForId(intentId: string): MessageIntent | null {
    return messageStateRef.current.intentsById[intentId] || null;
  }

  function setThreadRuntimeState(
    threadId: string,
    runtimeState: ThreadRuntimeState,
    options?: {
      activeIntentId?: string;
      remoteRunId?: string;
      error?: string;
    },
  ) {
    dispatchMessageState({
      type: "thread/runtime",
      threadId,
      runtimeState,
      activeIntentId: options?.activeIntentId,
      remoteRunId: options?.remoteRunId,
      error: options?.error,
    });
  }

  function publishTranscriptRunState(
    threadId: string,
    state: TranscriptRunState,
  ): TranscriptRunState {
    transcriptRunStateByThreadRef.current = {
      ...transcriptRunStateByThreadRef.current,
      [threadId]: state,
    };
    if (state.title) {
      applyThreadTitleUpdate(threadId, state.title);
    }
    const remoteRunId = state.activeRunId || undefined;
    if (state.busy) {
      const runtimeState: ThreadRuntimeState =
        state.activity === "reconciling"
          ? "reconciling_history"
          : "running_remote";
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
        dispatchMessageState({
          type: "thread/clear",
          threadId,
        });
        clearLiveStreamState(threadId);
      }
    }
    return state;
  }

  function syncTranscriptRunState(
    threadId: string,
    transcript: ThreadTranscript,
  ): TranscriptRunState {
    return publishTranscriptRunState(
      threadId,
      reduceTranscriptRunState(transcript.messages),
    );
  }

  function applyCommittedTranscriptRunState(
    event: Extract<DesktopChatStreamEvent, { type: "committed_message" }>,
  ): TranscriptRunState {
    const current =
      transcriptRunStateByThreadRef.current[event.threadId] ||
      reduceTranscriptRunState(
        transcriptSnapshotByThreadRef.current[event.threadId]?.messages || [],
      );
    return publishTranscriptRunState(
      event.threadId,
      applyTranscriptRunStateRecord(current, event.message, { seq: event.seq }),
    );
  }

  function updateLiveStreamState(
    threadId: string,
    updater: (current: LiveStreamState | null) => LiveStreamState | null,
  ): LiveStreamState | null {
    const next = updater(liveStreamStateRef.current[threadId] || null);
    const updated = { ...liveStreamStateRef.current };
    if (next) {
      updated[threadId] = next;
    } else {
      delete updated[threadId];
    }
    liveStreamStateRef.current = updated;
    setLiveStreamStateByThread(updated);
    return next;
  }

  function clearLiveStreamState(threadId: string) {
    updateLiveStreamState(threadId, () => null);
  }

  function getLiveStreamState(threadId: string): LiveStreamState | null {
    return liveStreamStateRef.current[threadId] || null;
  }

  function updateMessagesByThread(
    updater: (current: MessageMap) => MessageMap,
  ): MessageMap {
    const next = updater(messagesByThreadRef.current);
    messagesByThreadRef.current = next;
    setMessagesByThread(next);
    return next;
  }

  function updateRenderStateByThread(
    updater: (
      current: Record<string, RenderState>,
    ) => Record<string, RenderState>,
  ): void {
    const next = updater(renderStateByThreadRef.current);
    renderStateByThreadRef.current = next;
    setRenderStateByThread(next);
  }

  function applyThreadRenderState(threadId: string, renderState: RenderState) {
    const existing = renderStateByThreadRef.current[threadId];
    // Monotonic guard: drop late frames from a reconnect race so the rendered
    // snapshot never moves backward.
    if (existing && renderState.based_on_seq < existing.based_on_seq) {
      return;
    }
    updateRenderStateByThread((current) => ({
      ...current,
      [threadId]: renderState,
    }));
  }

  function applyThreadTitleUpdate(threadId: string, title: string) {
    const nextTitle = title.trim();
    if (!threadId || !nextTitle) {
      return;
    }

    threadTitleOverridesRef.current = {
      ...threadTitleOverridesRef.current,
      [threadId]: nextTitle,
    };

    setDesktopState((current) => {
      if (!current) {
        return current;
      }
      let changed = false;
      const updateThread = (
        thread: (typeof current.threads)[number],
      ): (typeof current.threads)[number] => {
        if (thread.id !== threadId || thread.title === nextTitle) {
          return thread;
        }
        changed = true;
        return { ...thread, title: nextTitle };
      };
      const threads = current.threads.map(updateThread);
      const sessions = current.sessions.map(updateThread);
      return changed ? { ...current, threads, sessions } : current;
    });

    if (selectedThreadIdRef.current === threadId && !editingThreadTitle) {
      setTitleDraft(nextTitle);
    }
  }

  function setRemotePendingInputs(
    threadId: string,
    pendingInputs: PendingThreadInput[],
  ) {
    setPendingRemoteInputsByThread((current) => {
      const next = { ...current };
      if (pendingInputs.length > 0) {
        next[threadId] = pendingInputs;
      } else {
        delete next[threadId];
      }
      return next;
    });
  }

  function rememberTranscriptSnapshot(
    threadId: string,
    transcript: ThreadTranscript,
    persist = true,
    syncRunState = true,
  ) {
    transcriptSnapshotByThreadRef.current = {
      ...transcriptSnapshotByThreadRef.current,
      [threadId]: transcript,
    };
    if (syncRunState) {
      syncTranscriptRunState(threadId, transcript);
    }
    if (persist) {
      const cacheTranscript = transcriptForCommittedCache(transcript);
      if (cacheTranscript.messages.length > 0 || !transcript.threadInfo?.activeRun) {
        // Persist the last render snapshot alongside committed messages so the
        // next cold/offline open can render folded history before a live frame.
        void window.garyxDesktop.saveThreadTranscriptCache(
          cacheTranscript,
          renderStateByThreadRef.current[threadId] ?? null,
        );
      }
    }
  }

  function applyCanonicalTranscript(
    threadId: string,
    transcript: ThreadTranscript,
    options?: { syncRunState?: boolean },
  ) {
    mirrorDualWrite(() =>
      mirror.applyAuthoritativeTranscript(threadId, transcript),
    );
    const resolvedTranscript = transcriptWithResolvedActiveRun(transcript);
    rememberTranscriptSnapshot(
      threadId,
      resolvedTranscript,
      true,
      options?.syncRunState ?? true,
    );
    setThreadInfoByThread((current) => ({
      ...current,
      [threadId]: resolvedTranscript.threadInfo ?? null,
    }));
    const visibleMessages = visibleTranscriptMessages(resolvedTranscript.messages);
    setRemotePendingInputs(threadId, resolvedTranscript.pendingInputs);
    startTransition(() => {
      updateMessagesByThread((current) => {
        const existing = current[threadId] || [];
        return {
          ...current,
          [threadId]: materializeRemoteTranscript(
            visibleMessages,
            existing,
          ),
        };
      });
    });
    markIntentsFromHistory(threadId, visibleMessages);
  }

  function handleChatStreamEvent(event: DesktopChatStreamEvent) {
    const threadId = event.threadId;
    if (event.type === "thread_render_frame") {
      // One atomic frame: apply the contiguous committed events through the
      // existing transport/ack path, then replace the render snapshot.
      for (const committed of event.events) {
        applyCommittedThreadMessage(committed);
      }
      applyThreadRenderState(threadId, event.renderState);
      return;
    }
    if (event.type !== "error") {
      return;
    }
    const currentStream = getLiveStreamState(threadId);
    const activeIntentId = currentStream?.activeIntentId;

    if (isThreadStreamGapError(event)) {
      if (activeIntentId) {
        dispatchMessageState({
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
          messagesByThreadRef.current[threadId] || [],
          activeIntentId,
          [currentStream?.assistantEntryId],
        )
      : { entries: [] as UiTranscriptMessage[], matched: false };
    const isTerminalRunError = event.terminal === true;
    if (
      !isTerminalRunError &&
      (isTransientGatewayErrorMessage(event.error) || recoveryResult.matched)
    ) {
      const recoveryStatusLabel = "Waiting to sync with gateway…";
      recordGatewayStatusObservation(
        {
          ok: false,
          bridgeReady: false,
          gatewayUrl: connection?.gatewayUrl || settingsDraft.gatewayUrl,
          error: event.error,
        },
        recoveryStatusLabel,
      );
      let assistantEntryId: string | null | undefined = null;
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
        dispatchMessageState({
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
      scheduleHistoryRefresh(threadId, 5, 1200, true);
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
      dispatchMessageState({
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
    setError(event.error);
  }

  function markIntentsFromHistory(
    threadId: string,
    transcript: TranscriptMessage[],
  ) {
    const visibleTranscript = visibleTranscriptMessages(transcript);
    const intents = Object.values(messageStateRef.current.intentsById).filter(
      (intent) => {
        return (
          intent.threadId === threadId &&
          [
            "dispatching",
            "remote_accepted",
            "awaiting_provider_ack",
            "awaiting_response",
            "awaiting_history",
          ].includes(intent.state)
        );
      },
    );

    for (const intent of intents) {
      const match = resolveIntentHistoryMatch(intent, visibleTranscript);
      if (!match.userVisible) {
        continue;
      }
      if (
        match.assistantVisible ||
        (!intent.responseText && intent.dispatchMode === "async_steer")
      ) {
        dispatchMessageState({
          type: "intent/completed",
          intentId: intent.intentId,
        });
      } else {
        dispatchMessageState({
          type: "intent/awaiting-history",
          intentId: intent.intentId,
          responseText: intent.responseText,
        });
      }
    }

    const runtime = selectThreadRuntime(messageStateRef.current, threadId);
    if (runtime && !hasPendingHistoryIntents(threadId)) {
      dispatchMessageState({
        type: "thread/clear",
        threadId,
      });
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

  function updateThreadHistoryPagination(
    threadId: string,
    updater: (
      current: ThreadHistoryPaginationState | null,
    ) => ThreadHistoryPaginationState | null,
  ) {
    const previous = historyPaginationByThreadRef.current[threadId] || null;
    const nextValue = updater(previous);
    const next = { ...historyPaginationByThreadRef.current };
    if (nextValue) {
      next[threadId] = nextValue;
    } else {
      delete next[threadId];
    }
    historyPaginationByThreadRef.current = next;
    setHistoryPaginationByThread(next);
  }

  function threadSummaryFromTranscript(
    threadId: string,
    transcript: ThreadTranscript,
  ): DesktopThreadSummary {
    if (transcript.thread) {
      return {
        ...transcript.thread,
        agentId: transcript.thread.agentId ?? transcript.threadInfo?.agentId ?? null,
        workspacePath:
          transcript.thread.workspacePath ?? transcript.threadInfo?.workspacePath ?? null,
        worktree: transcript.thread.worktree ?? transcript.threadInfo?.worktree ?? null,
        team: transcript.thread.team ?? transcript.team ?? null,
      };
    }

    const timestamps = transcript.messages
      .map((message) => message.timestamp || '')
      .filter(Boolean);
    const fallbackTimestamp =
      timestamps[timestamps.length - 1] || new Date().toISOString();
    const preview =
      transcript.messages.find((message) => message.text.trim())?.text.trim() || '';

    return {
      id: threadId,
      title: transcript.threadInfo?.agentId || threadId,
      createdAt: timestamps[0] || fallbackTimestamp,
      updatedAt: fallbackTimestamp,
      lastMessagePreview: preview,
      workspacePath: transcript.threadInfo?.workspacePath ?? null,
      messageCount: transcript.pageInfo?.totalMessages ?? transcript.messages.length,
      agentId: transcript.threadInfo?.agentId ?? null,
      recentRunId: transcript.threadInfo?.activeRun?.runId ?? null,
      worktree: transcript.threadInfo?.worktree ?? null,
      team: transcript.team ?? null,
    };
  }

  function cacheOpenableTranscriptThread(
    threadId: string,
    transcript: ThreadTranscript,
  ) {
    const summary = threadSummaryFromTranscript(threadId, transcript);
    setDesktopState((current) => {
      if (!current || current.threads.some((thread) => thread.id === threadId)) {
        return current;
      }
      // Hidden threads (side chats, child threads) live only in `sessions`,
      // so this cache write runs on every transcript application. Re-writing
      // an equivalent summary must keep `desktopState` identity stable, or
      // history-loading effects keyed on it re-fire and loop.
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

  function applyRemoteTranscript(
    threadId: string,
    transcript: ThreadTranscript,
    options?: {
      persist?: boolean;
      syncRunState?: boolean;
      /**
       * Batch 2b: set by the committed-stream path, whose events already
       * reached the mirror through ingest — dual-writing the folded
       * transcript again would apply the same data twice per event.
       */
      skipMirrorDualWrite?: boolean;
    },
  ) {
    if (!options?.skipMirrorDualWrite) {
      mirrorDualWrite(() => mirror.applyRemoteTranscript(threadId, transcript));
    }
    const resolvedTranscript = transcriptWithResolvedActiveRun(transcript);
    rememberTranscriptSnapshot(
      threadId,
      resolvedTranscript,
      options?.persist !== false,
      options?.syncRunState ?? true,
    );
    cacheOpenableTranscriptThread(threadId, resolvedTranscript);
    updateThreadHistoryPagination(threadId, (current) =>
      mergeRemotePaginationState(
        current,
        paginationStateFromTranscript(resolvedTranscript),
        messagesByThreadRef.current[threadId] || [],
      ),
    );
    setThreadInfoByThread((current) => ({
      ...current,
      [threadId]: resolvedTranscript.threadInfo ?? null,
    }));
    const visibleMessages = visibleTranscriptMessages(resolvedTranscript.messages);
    setRemotePendingInputs(threadId, resolvedTranscript.pendingInputs);
    startTransition(() => {
      updateMessagesByThread((current) => {
        const existing = current[threadId] || [];
        const merged = mergeRemoteTranscriptWithLocal(
          visibleMessages,
          existing,
          {
            activeRunLiveRows: Boolean(resolvedTranscript.threadInfo?.activeRun),
            preserveRemoteBeforeIndex:
              resolvedTranscript.pageInfo?.startIndex ?? null,
            threadRunActive: Boolean(resolvedTranscript.threadInfo?.activeRun),
            intentForId,
          },
        );
        if (
          merged.length === existing.length &&
          merged.every((entry, index) => entry === existing[index])
        ) {
          return current;
        }
        return {
          ...current,
          [threadId]: merged,
        };
      });
    });
    // Propagate the transcript's `team` block into `desktopState.threads[i]`
    // so team-bound threads render the team badge + sub-agent peek tabs as
    // soon as the thread metadata endpoint has confirmed the binding. Without
    // this merge, a list summary (which may have been fetched before the
    // first turn) could shadow the richer detail payload, leaving the UI
    // stuck on the plain agent label. Only write when the block is present
    // and different from what's already cached — idempotent updates must
    // not churn React identity and re-trigger dependent effects.
    if (resolvedTranscript.team !== undefined) {
      setDesktopState((current) => {
        if (!current) {
          return current;
        }
        const nextTeam = resolvedTranscript.team ?? null;
        let changed = false;
        const mapThreadTeam = (
          thread: (typeof current.threads)[number],
        ): (typeof current.threads)[number] => {
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
    markIntentsFromHistory(threadId, visibleMessages);
  }

  function applyOlderRemoteTranscriptPage(
    threadId: string,
    transcript: ThreadTranscript,
  ) {
    mirrorDualWrite(() => mirror.applyOlderHistoryPage(threadId, transcript));
    updateThreadHistoryPagination(threadId, () =>
      paginationStateFromTranscript(transcript),
    );
    const visibleMessages = visibleTranscriptMessages(transcript.messages);
    if (visibleMessages.length === 0) {
      return;
    }

    updateMessagesByThread((current) => {
      const existing = current[threadId] || [];
      const existingIds = new Set(existing.map((entry) => entry.id));
      const olderEntries = materializeRemoteTranscript(
        visibleMessages,
        [],
      ).filter((entry) => !existingIds.has(entry.id));
      if (olderEntries.length === 0) {
        return current;
      }
      return {
        ...current,
        [threadId]: [...olderEntries, ...existing],
      };
    });
  }

  async function loadOlderThreadHistoryPage(threadId: string) {
    const pagination = historyPaginationByThreadRef.current[threadId] || null;
    if (
      !pagination?.hasMoreBefore ||
      pagination.loadingBefore ||
      pagination.nextBeforeIndex === null
    ) {
      return;
    }

    updateThreadHistoryPagination(threadId, (current) => ({
      hasMoreBefore: Boolean(current?.hasMoreBefore),
      nextBeforeIndex: current?.nextBeforeIndex ?? null,
      loadingBefore: true,
    }));

    try {
      const transcript = await window.garyxDesktop.getThreadHistory({
        threadId,
        beforeIndex: pagination.nextBeforeIndex,
        limit: THREAD_HISTORY_PAGE_SIZE,
        userQueryLimit: THREAD_HISTORY_USER_QUERY_LIMIT,
      });
      const node = messagesRef.current;
      if (
        transcript.messages.length > 0 &&
        node &&
        selectedThreadIdRef.current === threadId
      ) {
        pendingMessagesPrependAnchorRef.current = {
          threadId,
          scrollHeight: node.scrollHeight,
          scrollTop: node.scrollTop,
        };
      }
      applyOlderRemoteTranscriptPage(threadId, transcript);
    } catch (historyError) {
      pendingMessagesPrependAnchorRef.current = null;
      setError(
        historyError instanceof Error
          ? historyError.message
          : "Failed to load earlier thread history",
      );
    } finally {
      if (selectedThreadIdRef.current !== threadId) {
        pendingMessagesPrependAnchorRef.current = null;
      }
      updateThreadHistoryPagination(threadId, (current) =>
        current ? { ...current, loadingBefore: false } : current,
      );
    }
  }

  function hasPendingHistoryIntents(threadId: string): boolean {
    return Object.values(messageStateRef.current.intentsById).some((intent) => {
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
    });
  }

  async function startCommittedThreadStream(
    threadId: string,
    transcript: ThreadTranscript,
    consumerId: string,
  ): Promise<void> {
    await window.garyxDesktop.startThreadStream({
      threadId,
      consumerId,
      afterSeq: streamResumeCursor({
        afterCursor: transcriptCommittedAfterCursor(transcript),
        fallbackMaxIndex: null,
      }),
    });
  }

  /// Incremental forward fetch for the selected thread. `authoritative: true`
  /// marks a full server refetch (no cache / reset / shrink / page-limit
  /// overflow) whose transcript must replace local state verbatim;
  /// `authoritative: false` marks an incremental aggregate that the caller
  /// must forward-merge onto the live snapshot, because the committed stream
  /// may have advanced it past this fetch's tail while pages were in flight.
  async function fetchSelectedThreadIncrementalTranscript(
    threadId: string,
    cached: ThreadTranscript | null,
    isCancelled: () => boolean,
  ): Promise<{ transcript: ThreadTranscript; authoritative: boolean }> {
    let current = cached;
    let cursor = transcriptCommittedAfterCursor(current);
    if (!current || cursor === null) {
      return {
        transcript: await window.garyxDesktop.getThreadHistory(threadId),
        authoritative: true,
      };
    }

    let pagesFetched = 0;
    let latestHasMoreAfter = false;
    for (
      let pageCount = 0;
      pageCount < THREAD_HISTORY_FORWARD_PAGE_LIMIT;
      pageCount += 1
    ) {
      const page = await window.garyxDesktop.getThreadHistory({
        threadId,
        afterIndex: cursor,
        limit: THREAD_HISTORY_PAGE_SIZE,
        userQueryLimit: THREAD_HISTORY_USER_QUERY_LIMIT,
      });
      if (isCancelled()) {
        return { transcript: current, authoritative: false };
      }
      pagesFetched = pageCount + 1;
      const action = decideTranscriptFetchPageAction({
        cursor,
        reset: page.pageInfo?.reset,
        hasMoreAfter: page.pageInfo?.hasMoreAfter,
        totalMessagesInThread: page.pageInfo?.totalMessages,
      });
      if (action.type === "reset" || action.type === "shrink_refetch") {
        await window.garyxDesktop.clearThreadTranscriptCache(threadId);
        return {
          transcript: await window.garyxDesktop.getThreadHistory(threadId),
          authoritative: true,
        };
      }

      current = mergeForwardTranscriptPage(current, page);
      latestHasMoreAfter = action.continuePaging;
      if (!action.continuePaging) {
        return { transcript: current, authoritative: false };
      }
      const nextCursor =
        page.pageInfo?.nextAfterIndex ?? transcriptCommittedAfterCursor(current);
      if (nextCursor === null || nextCursor <= cursor) {
        return { transcript: current, authoritative: false };
      }
      cursor = nextCursor;
    }
    if (isCancelled()) {
      return { transcript: current, authoritative: false };
    }
    if (
      shouldRefetchAuthoritativeAfterForwardPageLimit({
        pagesFetched,
        maxPages: THREAD_HISTORY_FORWARD_PAGE_LIMIT,
        hasMoreAfter: latestHasMoreAfter,
      })
    ) {
      await window.garyxDesktop.clearThreadTranscriptCache(threadId);
      return {
        transcript: await window.garyxDesktop.getThreadHistory(threadId),
        authoritative: true,
      };
    }
    return { transcript: current, authoritative: false };
  }

  async function loadSelectedThreadTranscriptFromSingleSource(
    threadId: string,
    isCancelled: () => boolean,
  ) {
    const hasRenderedThread = lastRenderedMessageThreadRef.current === threadId;
    const hasCachedMessages =
      (messagesByThreadRef.current[threadId] || []).length > 0;
    requestSelectedThreadMessagesBottomSnap(
      threadId,
      !hasRenderedThread || !hasCachedMessages,
    );

    setHistoryLoading(true);
    setError(null);
    let latestTranscript =
      transcriptSnapshotByThreadRef.current[threadId] || null;
    let streamReady = false;
    let streamStarted = false;
    try {
      const cached = await window.garyxDesktop.loadThreadTranscriptCache(threadId);
      if (isCancelled()) {
        return;
      }
      if (cached) {
        latestTranscript = cached.transcript;
        applyRemoteTranscript(threadId, cached.transcript, { persist: false });
        // Restore the offline render snapshot so folded history renders before
        // the live stream's first frame arrives.
        if (cached.renderState) {
          applyThreadRenderState(threadId, cached.renderState);
          // Batch 2b dual-write: the mirror's render snapshot only advances
          // through ingested frames, so replay the cached snapshot as a
          // synthesized snapshot-only frame (same wire semantics; the
          // monotonic guard applies on both sides).
          const cachedRenderState = cached.renderState;
          mirrorDualWrite(() =>
            mirror.ingest({
              type: "thread_render_frame",
              threadId,
              events: [],
              renderState: cachedRenderState,
            }),
          );
        }
        // Start the committed stream from the cached cursor right away: its
        // replay plus first render frame is what shows turns committed while
        // this client wasn't subscribed. Waiting for the incremental HTTP
        // fetch below kept the restored (possibly stale) render snapshot on
        // screen for the whole fetch, hiding those turns.
        await startCommittedThreadStream(
          threadId,
          cached.transcript,
          SELECTED_THREAD_STREAM_CONSUMER_ID,
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
      requestSelectedThreadMessagesBottomSnap(threadId, true);
      // The stream may have advanced the live snapshot past this fetch's tail
      // while pages were in flight; forward-merge keeps that progress. An
      // authoritative refetch (reset/shrink) intentionally replaces state.
      latestTranscript = fetched.authoritative
        ? fetched.transcript
        : mergeForwardTranscriptPage(
            transcriptSnapshotByThreadRef.current[threadId] ?? null,
            fetched.transcript,
          );
      applyRemoteTranscript(threadId, latestTranscript);
      if (transcriptHasAutomationResponse(latestTranscript.messages)) {
        setPendingAutomationRun(threadId, null);
      }
      streamReady = true;
    } catch (historyError) {
      if (!latestTranscript) {
        setError(
          historyError instanceof Error
            ? historyError.message
            : "Failed to load thread history",
        );
      } else {
        setError(
          historyError instanceof Error
            ? `Failed to sync latest thread history: ${historyError.message}`
            : "Failed to sync latest thread history",
        );
      }
    } finally {
      if (!isCancelled()) {
        setHistoryLoading(false);
        if (streamStarted || !streamReady || !latestTranscript) {
          return;
        }
        await startCommittedThreadStream(
          threadId,
          latestTranscript,
          SELECTED_THREAD_STREAM_CONSUMER_ID,
        );
      }
    }
  }

  function applyCommittedThreadMessage(
    event: Extract<DesktopChatStreamEvent, { type: "committed_message" }>,
  ) {
    const threadId = event.threadId;
    if (transcriptRewriteAction(event.message) === "refetch_authoritative") {
      void refetchAuthoritativeTranscriptAfterRewrite(threadId);
      return;
    }
    applyCommittedTranscriptRunState(event);
    const merged = committedMessageForwardPage(
      transcriptSnapshotByThreadRef.current[threadId] || null,
      event,
    );
    if (selectedThreadIdRef.current === threadId) {
      requestSelectedThreadMessagesBottomSnap(threadId, true);
    }
    applyRemoteTranscript(threadId, merged, {
      syncRunState: false,
      skipMirrorDualWrite: true,
    });
    const controlKind = transcriptControlKind(event.message);
    if (controlKind === "user_ack") {
      const control =
        event.message.content &&
        typeof event.message.content === "object" &&
        !Array.isArray(event.message.content)
          ? (event.message.content as { control?: Record<string, unknown> })
              .control
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

  function applyUserAck(
    threadId: string,
    runId: string,
    pendingInputId?: string,
  ) {
    let nextIntentId: string | undefined;
    const acknowledgedPendingInputId = pendingInputId?.trim() || "";
    updateLiveStreamState(threadId, (current) => {
      const pendingAckIntentIds = [...(current?.pendingAckIntentIds || [])];
      const matchedIndex = findPendingAckIntentIndex(
        pendingAckIntentIds,
        acknowledgedPendingInputId,
        messageStateRef.current.intentsById,
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
      dispatchMessageState({
        type: "intent/awaiting-history",
        intentId: nextIntentId,
        responseText: acknowledgedIntent?.responseText,
      });
      requestSelectedThreadMessagesBottomSnap(threadId, true);
      setThreadRuntimeState(threadId, "running_remote", {
        activeIntentId: nextIntentId,
        remoteRunId: runId,
      });
    }
  }

  function forceReleaseThreadRuntime(threadId: string) {
    const pendingStates = [
      "dispatching",
      "remote_accepted",
      "awaiting_provider_ack",
      "awaiting_response",
      "awaiting_history",
    ];
    for (const intent of Object.values(messageStateRef.current.intentsById)) {
      if (intent.threadId === threadId && pendingStates.includes(intent.state)) {
        dispatchMessageState({
          type: "intent/completed",
          intentId: intent.intentId,
        });
      }
    }
    dispatchMessageState({
      type: "thread/clear",
      threadId,
    });
    const liveStream = getLiveStreamState(threadId);
    if (
      liveStream &&
      ["reconciling", "disconnected", "failed"].includes(liveStream.streamStatus)
    ) {
      clearLiveStreamState(threadId);
    }
  }

  return {
    applyCanonicalTranscript,
    applyRemoteTranscript,
    clearLiveStreamState,
    forceReleaseThreadRuntime,
    getLiveStreamState,
    hasPendingHistoryIntents,
    intentForId,
    loadOlderThreadHistoryPage,
    messagesByThreadRef,
    setThreadRuntimeState,
    startCommittedThreadStream,
    threadTitleOverridesRef,
    updateLiveStreamState,
    updateMessagesByThread,
  };
}
