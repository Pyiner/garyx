import { useEffect, useState } from "react";

import type {
  DesktopChatStreamEvent,
  DesktopState,
  ThreadTranscript,
  TranscriptMessage,
} from "@shared/contracts";

import {
  type MessageIntent,
  type MessageMachineState,
  type ThreadRuntimeState,
} from "../message-machine";
import type {
  LiveStreamState,
  MessageMap,
  UiTranscriptMessage,
} from "./types";

export { SELECTED_THREAD_STREAM_CONSUMER_ID } from "../gateway-mirror/transcript-lifecycle";

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
  messagesNearEarlierUserTurnBoundary,
  type ThreadHistoryPaginationState,
} from "../gateway-mirror/transcript-materialize";
import type { GatewayMirror } from "../gateway-mirror/mirror";

type UseTranscriptControllerArgs = {
  activeHistoryPagination: ThreadHistoryPaginationState | null;
  activeMessages: UiTranscriptMessage[];
  activeThreadMessageKey: string | null;
  desktopState: DesktopState | null;
  historyLoading: boolean;
  liveStreamStateRef: React.MutableRefObject<Record<string, LiveStreamState>>;
  messageStateRef: React.MutableRefObject<MessageMachineState>;
  messagesRef: React.MutableRefObject<HTMLDivElement | null>;
  mirror: GatewayMirror;
  requestSelectedThreadMessagesBottomSnap: (
    threadId: string | null | undefined,
    forceStick?: boolean,
  ) => void;
  selectedThreadId: string | null;
  selectedThreadIdRef: React.MutableRefObject<string | null>;
};

export function useTranscriptController({
  activeHistoryPagination,
  activeMessages,
  activeThreadMessageKey,
  desktopState,
  historyLoading,
  liveStreamStateRef,
  messageStateRef,
  messagesRef,
  mirror,
  requestSelectedThreadMessagesBottomSnap,
  selectedThreadId,
  selectedThreadIdRef,
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
  // Batch 6b-2c: the lifecycle deps (including the side-chat stream
  // identity, which this hook cannot see) are fed by the AppShell wiring
  // layer next to setDispatchDeps.

  useEffect(() => {
    const listener = (event: DesktopChatStreamEvent) => {
      // Batch 6b-2c: the lifecycle owns the whole pass — mirror ingest
      // first (one atomic commit), then the machine/run-state/error side
      // effects.
      mirror.notifyStreamEvent(event);
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

    void mirror.loadSelectedThreadTranscript(selectedThreadId);

    return () => {
      mirror.cancelSelectedThreadLoad(selectedThreadId);
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

  // Batch 6b-2c: the older-page load (scroll-anchor capture + error
  // surfacing around the pure fetch/guard/apply) lives in the mirror's
  // transcript lifecycle.
  async function loadOlderThreadHistoryPage(threadId: string) {
    await mirror.loadOlderThreadHistoryPage(threadId);
  }

  function hasPendingHistoryIntents(threadId: string): boolean {
    return mirror.hasPendingHistoryIntents(threadId);
  }

  async function startCommittedThreadStream(
    threadId: string,
    transcript: ThreadTranscript,
    consumerId: string,
  ): Promise<void> {
    await mirror.startCommittedThreadStream(threadId, transcript, consumerId);
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
