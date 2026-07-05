import { useEffect, useRef, useState } from "react";

import type {
  ConnectionStatus,
  DesktopChatStreamEvent,
  DesktopSettings,
  DesktopState,
  ThreadTranscript,
  TranscriptMessage,
} from "@shared/contracts";
import {
  decideTranscriptFetchPageAction,
  isControlTranscriptMessage,
  isThreadStreamGapError,
  mergeForwardTranscriptPage,
  shouldRefetchAuthoritativeAfterForwardPageLimit,
  streamResumeCursor,
  transcriptCommittedAfterCursor,
  type TranscriptRunState,
} from "@shared/transcript-sync";

import {
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
import { isTransientGatewayErrorMessage } from "./gateway-errors";
import { extractImageGenerationImageContent } from "./image-generation-content";
import { isRunLoadingPlaceholderMessage } from "./loading-labels";
import type {
  LiveStreamState,
  MessageMap,
  PendingAutomationRun,
  UiTranscriptMessage,
} from "./types";

const THREAD_HISTORY_FORWARD_PAGE_LIMIT = 50;
export const SELECTED_THREAD_STREAM_CONSUMER_ID = "selected-thread";

// Batch 2a-1/2a-2: the pure materialization + remote-apply helpers live in
// gateway-mirror; re-export the public ones and import the internals the
// hook still uses.
export {
  isMissingThreadTranscript,
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
  messagesNearEarlierUserTurnBoundary,
  transcriptHasAutomationResponse,
  reconcileAssistantEntriesForGatewayRecovery,
  isMissingThreadTranscript,
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
  setPendingAutomationRun: (
    threadId: string,
    run: PendingAutomationRun | null,
  ) => void;
  /**
   * Batch 5b: remote title sync into the colocated title root — the root
   * applies its own not-editing guard.
   */
  syncThreadTitleDraft: (nextTitle: string) => void;
  settingsDraft: DesktopSettings;
};

export function useTranscriptController({
  activeHistoryPagination,
  activeMessages,
  activeThreadMessageKey,
  connection,
  desktopState,
  dispatchMessageState,
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
  setPendingAutomationRun,
  syncThreadTitleDraft,
  settingsDraft,
}: UseTranscriptControllerArgs) {
  // Batch 6a: the mirror is the single store for the render-side transcript
  // maps. This stable reader keeps the legacy `{ current }` shape for the
  // dispatch-orchestrator deps and the side-chat controller.
  const [messagesByThreadRef] = useState(() => ({
    get current(): MessageMap {
      return mirror.getTranscriptMapsSnapshot().messagesByThread as MessageMap;
    },
  }));
  // Batch 6b-2a: run-state and title overrides live in the mirror's
  // transcript lifecycle; this stable reader keeps the `{ current }`
  // shape for the dispatch-orchestrator deps (the 6a reader pattern).
  const [threadTitleOverridesRef] = useState(() => ({
    get current(): Record<string, string> {
      return mirror.getThreadTitleOverrides();
    },
  }));
  const streamEventHandlerRef = useRef<(event: DesktopChatStreamEvent) => void>(
    () => {},
  );

  useEffect(() => {
    streamEventHandlerRef.current = handleChatStreamEvent;
  });

  // Batch 6b-2a: the transcript lifecycle's React seams, refreshed every
  // commit (the streamEventHandlerRef pattern).
  useEffect(() => {
    mirror.setTranscriptLifecycleDeps({
      setDesktopState,
      syncThreadTitleDraft,
      requestSelectedThreadMessagesBottomSnap,
      selectedThreadIdRef,
      liveStreamStateRef,
      refetchAuthoritativeTranscriptAfterRewrite,
    });
  });

  useEffect(() => {
    const listener = (event: DesktopChatStreamEvent) => {
      // The mirror is the transcript store: frames commit atomically inside
      // ingest. The handler below keeps owning machine/live-stream/error
      // side effects.
      mirror.ingest(event);
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
    mirror.setThreadRuntimeState(threadId, runtimeState, options);
  }

  // Batch 6b-2a: the run-state chain (publish/sync/committed-apply, title
  // propagation) lives in the mirror's transcript lifecycle; these are
  // thin delegates keeping the hook wiring unchanged until 2b/2c.
  function syncTranscriptRunState(
    threadId: string,
    transcript: ThreadTranscript,
  ): TranscriptRunState {
    return mirror.syncTranscriptRunState(threadId, transcript);
  }

  function applyCommittedTranscriptRunState(
    event: Extract<DesktopChatStreamEvent, { type: "committed_message" }>,
  ): TranscriptRunState {
    return mirror.applyCommittedTranscriptRunState(event);
  }

  // Batch 3c-1: the mirror owns live-stream storage. These proxies keep
  // `liveStreamStateRef` as the synchronous shadow for event-path readers
  // (the mirror's notify never runs render code synchronously, so the ref
  // assignment right after the mirror call is not observable in between).
  function updateLiveStreamState(
    threadId: string,
    updater: (current: LiveStreamState | null) => LiveStreamState | null,
  ): LiveStreamState | null {
    const next = mirror.updateThreadLiveStream(threadId, updater);
    liveStreamStateRef.current = mirror.getLiveStreamMap();
    return next;
  }

  function replaceLiveStreamThreadId(fromThreadId: string, toThreadId: string) {
    mirror.replaceLiveStreamThreadId(fromThreadId, toThreadId);
    liveStreamStateRef.current = mirror.getLiveStreamMap();
  }

  function clearLiveStreamState(threadId: string) {
    updateLiveStreamState(threadId, () => null);
  }

  function getLiveStreamState(threadId: string): LiveStreamState | null {
    return liveStreamStateRef.current[threadId] || null;
  }

  /**
   * Batch 6a: the mirror's message cache is the single message store.
   * Local optimistic/recovery writes still run through this legacy-shaped
   * updater; per-thread diffs commit into the mirror, which notifies the
   * read side. Remote applies never come through here — the mirror
   * computes those itself (applyRemote/applyAuthoritative/applyOlderPage).
   */
  // Batch 6b-2b: the apply chain (persist, session cache, title/team
  // propagation, intent marking) lives in the mirror's transcript
  // lifecycle behind the accept* high-level entries; these are thin
  // delegates keeping the hook wiring unchanged until 2c/2d.
  function updateMessagesByThread(
    updater: (current: MessageMap) => MessageMap,
  ): MessageMap {
    return mirror.updateMessagesByThread(updater);
  }

  function applyCanonicalTranscript(
    threadId: string,
    transcript: ThreadTranscript,
    options?: { syncRunState?: boolean },
  ) {
    mirror.acceptAuthoritativeTranscript(threadId, transcript, options);
  }

  function handleChatStreamEvent(event: DesktopChatStreamEvent) {
    const threadId = event.threadId;
    if (event.type === "thread_render_frame") {
      // The mirror.ingest call at the stream listener already committed the
      // frame (events + render snapshot, monotonic guard included). This
      // pass keeps the per-event machine/run-state/ack side effects.
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
    mirror.markIntentsFromHistory(threadId, transcript);
  }

  function applyRemoteTranscript(
    threadId: string,
    transcript: ThreadTranscript,
    options?: {
      persist?: boolean;
      syncRunState?: boolean;
      /**
       * Set by the committed-stream path, whose events already reached the
       * mirror through ingest — applying the folded transcript again would
       * apply the same data twice per event.
       */
      mirrorAlreadyApplied?: boolean;
    },
  ) {
    mirror.acceptRemoteTranscript(threadId, transcript, options);
  }

  async function loadOlderThreadHistoryPage(threadId: string) {
    // Batch 6a: the mirror owns the older-page fetch (pagination guard,
    // loadingBefore lifecycle, page apply). The hook keeps the UI-owned
    // scroll-anchor capture between fetch and apply, and error surfacing.
    try {
      await mirror.loadOlderThreadHistoryPage(threadId, {
        onPageFetched: (transcript) => {
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
        },
      });
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
    }
  }

  function hasPendingHistoryIntents(threadId: string): boolean {
    return mirror.hasPendingHistoryIntents(threadId);
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
    let latestTranscript: ThreadTranscript | null =
      mirror.getThreadSnapshotTranscript(threadId);
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
        // the live stream's first frame arrives. The mirror's render snapshot
        // only advances through ingested frames, so replay the cached snapshot
        // as a synthesized snapshot-only frame (same wire semantics; the
        // monotonic guard applies).
        if (cached.renderState) {
          mirror.ingest({
            type: "thread_render_frame",
            threadId,
            events: [],
            renderState: cached.renderState,
          });
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
      // Batch 4b: a selected-but-missing thread (externally entered
      // #/thread/<id> that stays addressable) must not be applied or
      // streamed — the gateway history responds remoteFound:false with an
      // empty transcript, and the stream endpoint would 404-retry forever.
      // The predicate is shared with ensureThreadOpenable. It gates on the
      // AUTHORITATIVE fetch result, so a stale persisted cache for a
      // deleted thread lands here too: the cached fast path above already
      // applied it and started the stream, so roll both back — stop the
      // stream, drop the persisted cache, and clear the applied state
      // (mirror.clearThreadTranscript resets all five transcript maps plus
      // the committed records/frontier in one commit).
      if (
        fetched.authoritative &&
        isMissingThreadTranscript(fetched.transcript)
      ) {
        if (streamStarted) {
          await window.garyxDesktop.stopThreadStream({
            threadId,
            consumerId: SELECTED_THREAD_STREAM_CONSUMER_ID,
          });
          streamStarted = false;
        }
        if (latestTranscript) {
          void window.garyxDesktop.clearThreadTranscriptCache(threadId);
          mirror.clearThreadTranscript(threadId);
          latestTranscript = null;
        }
        setError(`Thread not found: ${threadId}`);
        return;
      }
      requestSelectedThreadMessagesBottomSnap(threadId, true);
      // The stream may have advanced the live snapshot past this fetch's tail
      // while pages were in flight; forward-merge keeps that progress. An
      // authoritative refetch (reset/shrink) intentionally replaces state.
      latestTranscript = fetched.authoritative
        ? fetched.transcript
        : mergeForwardTranscriptPage(
            mirror.getThreadSnapshotTranscript(threadId),
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
    mirror.applyCommittedThreadMessage(event);
  }

  function applyUserAck(
    threadId: string,
    runId: string,
    pendingInputId?: string,
  ) {
    mirror.applyUserAck(threadId, runId, pendingInputId);
  }

  function forceReleaseThreadRuntime(threadId: string) {
    mirror.forceReleaseThreadRuntime(threadId);
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
    // Mirror-backed `{ current }` reader (batch 6a) for the dispatch
    // orchestrator deps and the side-chat controller.
    messagesByThreadRef,
    replaceLiveStreamThreadId,
    setThreadRuntimeState,
    startCommittedThreadStream,
    threadTitleOverridesRef,
    updateLiveStreamState,
    updateMessagesByThread,
  };
}
