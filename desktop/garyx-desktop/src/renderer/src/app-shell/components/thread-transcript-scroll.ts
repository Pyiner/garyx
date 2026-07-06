// Thread-transcript scroll on top of the shadcn MessageScroller primitive.
// The primitive owns the mechanics: stick-to-bottom while streaming
// (following-bottom / free-scrolling mode machine + resize observers),
// prepend anchoring for older-history pages (preserveScrollOnPrepend),
// and the floating scroll-to-end button. What remains here is Garyx
// wiring: the shell-owned scroll INTENT bundle (pending bottom snaps,
// stick/force flags, prepend bookkeeping) consumed inside the provider
// by TranscriptScrollBridge, plus the scroll-triggered older-page loads.
// The intent refs stay in the AppShell shell — they must survive
// viewport unmounts: automations pre-arm a bottom snap from the
// automation view, and dispatch/lifecycle orchestration requests snaps
// regardless of the active view.

import { useContext, useEffect, useLayoutEffect } from "react";

import { useMessageScroller } from "@/components/ui/message-scroller";
import { GatewayMirrorContext } from "../../gateway-mirror/react";
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
 * work from any view; the viewport bridge consumes them while mounted.
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
  // Nullable on purpose: Storybook mounts ThreadPage without the gateway
  // provider (and without a scrollIntent). Every mirror consumer below is
  // on a scrollIntent-gated path, so a bare mount is a complete no-op.
  const mirror = useContext(GatewayMirrorContext);

  // Scroll-triggered older-page auto-load (the near-boundary probe and
  // the fetch trigger are one feature with the scroll handler below).
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
      if (mirror && scrollIntent.selectedThreadIdRef.current === threadId) {
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

  function handleMessagesScroll() {
    if (!scrollIntent) {
      return;
    }
    const node = messagesRef.current;
    scrollIntent.shouldStickMessagesToBottomRef.current =
      messagesNearBottom(node);
    const selectedThreadId = scrollIntent.selectedThreadIdRef.current;
    if (
      mirror &&
      selectedThreadId &&
      node &&
      messagesNearEarlierUserTurnBoundary(node)
    ) {
      void mirror.loadOlderThreadHistoryPage(selectedThreadId);
    }
  }

  function handleMessagesUserScrollIntent() {
    if (!scrollIntent) {
      return;
    }
    // A real user gesture cancels any armed force-snap so the viewport
    // does not fight the user; the MessageScroller primitive drops out
    // of following-bottom mode on its own.
    scrollIntent.forceMessagesBottomSnapRef.current = false;
  }

  if (!scrollIntent) {
    return null;
  }
  return {
    onMessagesScroll: handleMessagesScroll,
    onMessagesUserScrollIntent: handleMessagesUserScrollIntent,
  };
}

type TranscriptScrollBridgeProps = {
  activeMessages: UiTranscriptMessage[];
  activeThreadMessageKey: string | null;
  historyLoading: boolean;
  scrollIntent: TranscriptScrollIntent | null;
};

/**
 * Consumes the shell's scroll intents from inside the MessageScroller
 * provider: thread-switch and requested bottom snaps run through the
 * primitive's scrollToEnd (which also re-arms following-bottom mode),
 * while prepend anchors are simply retired because the viewport already
 * restored the position via preserveScrollOnPrepend. Renders nothing.
 */
export function TranscriptScrollBridge({
  activeMessages,
  activeThreadMessageKey,
  historyLoading,
  scrollIntent,
}: TranscriptScrollBridgeProps): null {
  const { scrollToEnd } = useMessageScroller();

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
        shouldStickMessagesToBottomRef.current = false;
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
      scrollToEnd({ behavior: "auto" });
      pendingThreadBottomSnapRef.current = null;
      forceMessagesBottomSnapRef.current = false;
      if (threadChanged || forceSnap) {
        shouldStickMessagesToBottomRef.current = true;
      }
    } else if (
      currentThreadId &&
      !historyLoading &&
      tailChanged &&
      shouldStickMessagesToBottomRef.current
    ) {
      scrollToEnd({ behavior: "auto" });
    } else if (pendingSnapMatches && currentCount > 0 && !historyLoading) {
      pendingThreadBottomSnapRef.current = null;
      forceMessagesBottomSnapRef.current = false;
    }

    lastRenderedMessageThreadRef.current = currentThreadId;
    lastRenderedMessageCountRef.current = currentCount;
    lastRenderedMessageTailSignatureRef.current = currentTailSignature;
  }, [activeThreadMessageKey, activeMessages, historyLoading, scrollIntent, scrollToEnd]);

  return null;
}
