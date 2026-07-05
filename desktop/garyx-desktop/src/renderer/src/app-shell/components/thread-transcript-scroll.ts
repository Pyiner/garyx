// Thread-transcript scroll colocation (endgame batch 5b): the DOM-bound
// scroll effects, the stick-to-bottom scheduler, and the container
// handlers move verbatim from useMessagesScrollController/AppShell into
// this ThreadPage-owned hook. The scroll INTENT state (pending bottom
// snaps, stick/force flags, prepend anchor, last-rendered bookkeeping)
// stays in the AppShell shell inside a TranscriptScrollIntent bundle —
// it must survive viewport unmounts: automations pre-arm a bottom snap
// from the automation view, and the dispatch/lifecycle orchestration
// requests snaps regardless of the active view.

import { useEffect, useLayoutEffect, useRef } from "react";

import { useGatewayMirror } from "../../gateway-mirror/react";
import { messagesNearEarlierUserTurnBoundary } from "../../gateway-mirror/transcript-materialize";
import type { ThreadHistoryPaginationState } from "../../gateway-mirror/transcript-materialize";
import type { UiTranscriptMessage } from "../types";

const MESSAGES_BOTTOM_THRESHOLD_PX = 48;

export function messagesNearBottom(node: HTMLDivElement | null): boolean {
  if (!node) {
    return true;
  }
  return (
    node.scrollHeight - node.scrollTop - node.clientHeight <
    MESSAGES_BOTTOM_THRESHOLD_PX
  );
}

export function scrollMessagesToLatest(
  node: HTMLDivElement | null,
  behavior: ScrollBehavior = "auto",
) {
  node?.scrollTo({
    top: node.scrollHeight,
    behavior,
  });
}

export function messageTailSignature(messages: UiTranscriptMessage[]): string {
  const lastMessage = messages[messages.length - 1];
  if (!lastMessage) {
    return "0";
  }
  return [
    messages.length,
    lastMessage.id,
    lastMessage.role,
    lastMessage.text.length,
    lastMessage.pending ? "1" : "0",
    lastMessage.localState || "",
  ].join(":");
}

/**
 * The shell-owned scroll intent bundle. Plain refs so writers (the
 * dispatch/lifecycle orchestration, automations, the shell's snap API)
 * work from any view; the viewport hook consumes them while mounted.
 */
export interface TranscriptScrollIntent {
  pendingThreadBottomSnapRef: { current: string | null };
  forceMessagesBottomSnapRef: { current: boolean };
  shouldStickMessagesToBottomRef: { current: boolean };
  pendingMessagesPrependAnchorRef: {
    current: {
      threadId: string;
      scrollHeight: number;
      scrollTop: number;
    } | null;
  };
  lastRenderedMessageThreadRef: { current: string | null };
  lastRenderedMessageCountRef: { current: number };
  lastRenderedMessageTailSignatureRef: { current: string };
  selectedThreadIdRef: { readonly current: string | null };
}

export type ThreadTranscriptScrollHandlers = {
  onMessagesScroll: () => void;
  onMessagesUserScrollIntent: () => void;
};

type UseThreadTranscriptScrollArgs = {
  activeHistoryPagination: ThreadHistoryPaginationState | null;
  activeMessages: UiTranscriptMessage[];
  activeThreadMessageKey: string | null;
  historyLoading: boolean;
  messagesRef: React.RefObject<HTMLDivElement | null>;
  /**
   * Absent for the side-chat ThreadPage instance, which keeps its own
   * lightweight scroll handlers (no stick/snap machinery): every effect
   * gates to a no-op and the hook returns null handlers.
   */
  scrollIntent: TranscriptScrollIntent | null;
};

export function useThreadTranscriptScroll({
  activeHistoryPagination,
  activeMessages,
  activeThreadMessageKey,
  historyLoading,
  messagesRef,
  scrollIntent,
}: UseThreadTranscriptScrollArgs): ThreadTranscriptScrollHandlers | null {
  const mirror = useGatewayMirror();
  // Scheduler-internal state (frame/timeout bookkeeping and the force
  // budget). Viewport-local: a fresh mount starts with clean scheduling.
  const messagesStickScrollFrameRef = useRef<number | null>(null);
  const messagesStickScrollTimeoutsRef = useRef<number[]>([]);
  const messagesStickScrollGenerationRef = useRef(0);
  const messagesForceScrollBudgetRef = useRef(false);

  useEffect(() => {
    return () => {
      if (messagesStickScrollFrameRef.current !== null) {
        window.cancelAnimationFrame(messagesStickScrollFrameRef.current);
        messagesStickScrollFrameRef.current = null;
      }
    };
  }, []);

  useLayoutEffect(() => {
    if (!scrollIntent) {
      return;
    }
    const {
      forceMessagesBottomSnapRef,
      lastRenderedMessageCountRef,
      lastRenderedMessageTailSignatureRef,
      lastRenderedMessageThreadRef,
      pendingMessagesPrependAnchorRef,
      pendingThreadBottomSnapRef,
      shouldStickMessagesToBottomRef,
    } = scrollIntent;
    const currentThreadId = activeThreadMessageKey;
    const currentCount = activeMessages.length;
    const currentTailSignature = messageTailSignature(activeMessages);
    const prependAnchor = pendingMessagesPrependAnchorRef.current;
    if (prependAnchor) {
      if (prependAnchor.threadId === currentThreadId) {
        const node = messagesRef.current;
        if (node) {
          node.scrollTop =
            node.scrollHeight -
            prependAnchor.scrollHeight +
            prependAnchor.scrollTop;
          shouldStickMessagesToBottomRef.current = false;
        }
      }
      pendingMessagesPrependAnchorRef.current = null;
      lastRenderedMessageThreadRef.current = currentThreadId;
      lastRenderedMessageCountRef.current = currentCount;
      lastRenderedMessageTailSignatureRef.current = currentTailSignature;
      return;
    }
    const previousThreadId = lastRenderedMessageThreadRef.current;
    const previousTailSignature = lastRenderedMessageTailSignatureRef.current;
    const threadChanged = currentThreadId !== previousThreadId;
    const tailChanged = currentTailSignature !== previousTailSignature;
    const pendingSnapMatches =
      pendingThreadBottomSnapRef.current === currentThreadId;
    const forceSnap =
      pendingSnapMatches && forceMessagesBottomSnapRef.current;
    const shouldSnapToBottom = Boolean(
      currentThreadId &&
      currentCount > 0 &&
      !historyLoading &&
      (threadChanged ||
        forceSnap ||
        (pendingSnapMatches && shouldStickMessagesToBottomRef.current)),
    );

    if (shouldSnapToBottom) {
      scheduleMessagesScrollToLatest("auto", { force: forceSnap });
      pendingThreadBottomSnapRef.current = null;
      if (threadChanged || forceSnap) {
        shouldStickMessagesToBottomRef.current = true;
      }
    } else if (
      currentThreadId &&
      !historyLoading &&
      tailChanged &&
      shouldStickMessagesToBottomRef.current
    ) {
      scheduleMessagesScrollToLatest("auto");
    } else if (pendingSnapMatches && currentCount > 0 && !historyLoading) {
      pendingThreadBottomSnapRef.current = null;
      forceMessagesBottomSnapRef.current = false;
    }

    lastRenderedMessageThreadRef.current = currentThreadId;
    lastRenderedMessageCountRef.current = currentCount;
    lastRenderedMessageTailSignatureRef.current = currentTailSignature;
  }, [activeThreadMessageKey, activeMessages, historyLoading]);

  useEffect(() => {
    const node = messagesRef.current;
    if (!scrollIntent || !node || !activeThreadMessageKey) {
      return;
    }

    const scrollIfSticky = () => {
      if (scrollIntent.shouldStickMessagesToBottomRef.current) {
        scheduleMessagesScrollToLatest("auto");
      }
    };
    const resizeObserver =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(scrollIfSticky);
    const observedChildren = new Set<Element>();

    const syncObservedChildren = () => {
      if (!resizeObserver) {
        return;
      }
      resizeObserver.observe(node);
      for (const child of Array.from(node.children)) {
        if (observedChildren.has(child)) {
          continue;
        }
        observedChildren.add(child);
        resizeObserver.observe(child);
      }
      for (const child of Array.from(observedChildren)) {
        if (child.parentElement !== node) {
          observedChildren.delete(child);
          resizeObserver.unobserve(child);
        }
      }
    };

    syncObservedChildren();
    const mutationObserver = new MutationObserver(() => {
      syncObservedChildren();
      scrollIfSticky();
    });
    mutationObserver.observe(node, { childList: true });

    return () => {
      mutationObserver.disconnect();
      resizeObserver?.disconnect();
    };
  }, [activeThreadMessageKey]);

  // Scroll-triggered older-page auto-load (moved with the handlers: the
  // near-boundary probe and the fetch trigger are one feature).
  useEffect(() => {
    if (
      !scrollIntent ||
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
      if (scrollIntent.selectedThreadIdRef.current === threadId) {
        void mirror.loadOlderThreadHistoryPage(threadId);
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

  function cancelMessagesScrollCallbacks() {
    if (messagesStickScrollFrameRef.current !== null) {
      window.cancelAnimationFrame(messagesStickScrollFrameRef.current);
      messagesStickScrollFrameRef.current = null;
    }
    for (const timeout of messagesStickScrollTimeoutsRef.current) {
      window.clearTimeout(timeout);
    }
    messagesStickScrollTimeoutsRef.current = [];
  }

  function cancelMessagesForceScrollBudget() {
    if (!scrollIntent) {
      return;
    }
    messagesStickScrollGenerationRef.current += 1;
    cancelMessagesScrollCallbacks();
    messagesForceScrollBudgetRef.current = false;
    scrollIntent.forceMessagesBottomSnapRef.current = false;
  }

  function scheduleMessagesScrollToLatest(
    behavior: ScrollBehavior = "auto",
    options?: {
      force?: boolean;
    },
  ) {
    if (!scrollIntent) {
      return;
    }
    const { forceMessagesBottomSnapRef, shouldStickMessagesToBottomRef } =
      scrollIntent;
    const forceBudget = Boolean(
      options?.force ||
        forceMessagesBottomSnapRef.current ||
        messagesForceScrollBudgetRef.current,
    );
    const generation = messagesStickScrollGenerationRef.current + 1;
    messagesStickScrollGenerationRef.current = generation;
    cancelMessagesScrollCallbacks();
    if (forceBudget) {
      shouldStickMessagesToBottomRef.current = true;
      forceMessagesBottomSnapRef.current = true;
      messagesForceScrollBudgetRef.current = true;
    }

    const runAttempt = (index: number) => {
      if (generation !== messagesStickScrollGenerationRef.current) {
        return;
      }
      const forceActive =
        forceMessagesBottomSnapRef.current ||
        messagesForceScrollBudgetRef.current;
      if (forceActive || shouldStickMessagesToBottomRef.current) {
        scrollMessagesToLatest(messagesRef.current, index === 0 ? behavior : "auto");
      }
      if (forceBudget && index >= 4) {
        messagesForceScrollBudgetRef.current = false;
        forceMessagesBottomSnapRef.current = false;
      }
    };

    runAttempt(0);
    messagesStickScrollFrameRef.current = window.requestAnimationFrame(() => {
      messagesStickScrollFrameRef.current = null;
      runAttempt(1);
    });
    if (forceBudget) {
      for (const [index, delayMs] of [40, 120, 260].entries()) {
        const timeout = window.setTimeout(() => {
          messagesStickScrollTimeoutsRef.current =
            messagesStickScrollTimeoutsRef.current.filter((value) => value !== timeout);
          runAttempt(index + 2);
        }, delayMs);
        messagesStickScrollTimeoutsRef.current.push(timeout);
      }
    }
  }

  function handleMessagesScroll() {
    if (!scrollIntent) {
      return;
    }
    const node = messagesRef.current;
    scrollIntent.shouldStickMessagesToBottomRef.current =
      messagesNearBottom(node);
    const selectedThreadId = scrollIntent.selectedThreadIdRef.current;
    if (selectedThreadId && node && messagesNearEarlierUserTurnBoundary(node)) {
      void mirror.loadOlderThreadHistoryPage(selectedThreadId);
    }
  }

  if (!scrollIntent) {
    return null;
  }
  return {
    onMessagesScroll: handleMessagesScroll,
    onMessagesUserScrollIntent: cancelMessagesForceScrollBudget,
  };
}
