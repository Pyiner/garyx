import { useEffect, useLayoutEffect, useRef } from "react";

import type { UiTranscriptMessage } from "./types";

type UseMessagesScrollControllerArgs = {
  activeMessages: UiTranscriptMessage[];
  activeThreadMessageKey: string | null;
  historyLoading: boolean;
  messageTailSignature: (messages: UiTranscriptMessage[]) => string;
  pendingThreadBottomSnapRef: React.MutableRefObject<string | null>;
  scrollMessagesToLatest: (
    node: HTMLDivElement | null,
    behavior?: ScrollBehavior,
  ) => void;
  selectedThreadIdRef: React.MutableRefObject<string | null>;
};

export function useMessagesScrollController({
  activeMessages,
  activeThreadMessageKey,
  historyLoading,
  messageTailSignature,
  pendingThreadBottomSnapRef,
  scrollMessagesToLatest,
  selectedThreadIdRef,
}: UseMessagesScrollControllerArgs) {
  const messagesRef = useRef<HTMLDivElement | null>(null);
  const pendingMessagesPrependAnchorRef = useRef<{
    threadId: string;
    scrollHeight: number;
    scrollTop: number;
  } | null>(null);
  const forceMessagesBottomSnapRef = useRef(false);
  const shouldStickMessagesToBottomRef = useRef(true);
  const messagesStickScrollFrameRef = useRef<number | null>(null);
  const messagesStickScrollTimeoutsRef = useRef<number[]>([]);
  const messagesStickScrollGenerationRef = useRef(0);
  const messagesForceScrollBudgetRef = useRef(false);
  const lastRenderedMessageThreadRef = useRef<string | null>(null);
  const lastRenderedMessageCountRef = useRef(0);
  const lastRenderedMessageTailSignatureRef = useRef("0");

  useEffect(() => {
    return () => {
      if (messagesStickScrollFrameRef.current !== null) {
        window.cancelAnimationFrame(messagesStickScrollFrameRef.current);
        messagesStickScrollFrameRef.current = null;
      }
    };
  }, []);

  useLayoutEffect(() => {
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
    if (!node || !activeThreadMessageKey) {
      return;
    }

    const scrollIfSticky = () => {
      if (shouldStickMessagesToBottomRef.current) {
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

  useEffect(() => {
    if (activeThreadMessageKey == null) {
      pendingThreadBottomSnapRef.current = null;
      forceMessagesBottomSnapRef.current = false;
      return;
    }
    requestMessagesBottomSnap(activeThreadMessageKey, true);
  }, [activeThreadMessageKey]);

  function requestMessagesBottomSnap(
    threadId: string | null | undefined,
    forceStick = false,
  ) {
    if (!threadId) {
      return;
    }
    pendingThreadBottomSnapRef.current = threadId;
    if (forceStick) {
      shouldStickMessagesToBottomRef.current = true;
      forceMessagesBottomSnapRef.current = true;
    }
  }

  function requestSelectedThreadMessagesBottomSnap(
    threadId: string | null | undefined,
    forceStick = false,
  ) {
    if (!threadId || threadId !== selectedThreadIdRef.current) {
      return;
    }
    requestMessagesBottomSnap(threadId, forceStick);
  }

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
    messagesStickScrollGenerationRef.current += 1;
    cancelMessagesScrollCallbacks();
    messagesForceScrollBudgetRef.current = false;
    forceMessagesBottomSnapRef.current = false;
  }

  function scheduleMessagesScrollToLatest(
    behavior: ScrollBehavior = "auto",
    options?: {
      force?: boolean;
    },
  ) {
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

  return {
    cancelMessagesForceScrollBudget,
    lastRenderedMessageThreadRef,
    messagesRef,
    pendingMessagesPrependAnchorRef,
    requestMessagesBottomSnap,
    requestSelectedThreadMessagesBottomSnap,
    shouldStickMessagesToBottomRef,
  };
}
