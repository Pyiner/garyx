// Thread-transcript scroll on top of the shadcn MessageScroller primitive.
// The primitive owns the mechanics: stick-to-bottom while streaming
// (following-bottom / free-scrolling mode machine + resize observers),
// prepend anchoring for older-history pages (preserveScrollOnPrepend),
// and the floating scroll-to-end button. What remains here is Garyx
// wiring: the shell-owned scroll INTENT bundle (pending bottom snaps,
// stick/force flags, prepend bookkeeping) consumed inside the provider
// by TranscriptScrollCoordinator, plus the scroll-triggered older-page loads.
// The intent refs stay in the AppShell shell — they must survive
// viewport unmounts: automations pre-arm a bottom snap from the
// automation view, and dispatch/lifecycle orchestration requests snaps
// regardless of the active view.

import { useContext, useEffect, useLayoutEffect, useRef } from "react";

import { useMessageScroller } from "@/components/ui/message-scroller";
import { GatewayMirrorContext } from "../../gateway-mirror/react";
import { messagesNearEarlierUserTurnBoundary } from "../../gateway-mirror/transcript-materialize";
import type { ThreadHistoryPaginationState } from "../../gateway-mirror/transcript-materialize";
import type { UiTranscriptMessage } from "../types";
import {
  tailThinkingScrollReserve,
  type TranscriptScrollAnchorSnapshot,
} from "./transcript-scroll-anchor";
import {
  applyTranscriptScrollTransaction,
  beginTranscriptScrollTransaction,
  decideTranscriptBottomScroll,
  settleTranscriptScrollTransaction,
  type TranscriptScrollTransaction,
} from "./transcript-scroll-transaction";
export { messageTailSignature } from "./transcript-scroll-transaction";

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

const MESSAGE_SCROLLER_CONTENT_SELECTOR =
  '[data-slot="message-scroller-content"]';
const MESSAGE_SCROLLER_ITEM_SELECTOR = '[data-slot="message-scroller-item"]';
const TAIL_THINKING_ROW_SELECTOR = '[data-tail-thinking-row="true"]';
const TAIL_THINKING_RESERVE_PROPERTY = "--messages-tail-scroll-reserve";

function transcriptContent(viewport: HTMLElement): HTMLElement | null {
  return viewport.querySelector<HTMLElement>(MESSAGE_SCROLLER_CONTENT_SELECTOR);
}

function captureTranscriptScrollAnchor(
  viewport: HTMLElement,
): TranscriptScrollAnchorSnapshot | null {
  const content = transcriptContent(viewport);
  if (!content) {
    return null;
  }
  const viewportRect = viewport.getBoundingClientRect();
  const anchor = Array.from(
    content.querySelectorAll<HTMLElement>(MESSAGE_SCROLLER_ITEM_SELECTOR),
  ).find((item) => {
    const rect = item.getBoundingClientRect();
    return (
      rect.bottom > viewportRect.top + 1 &&
      rect.top < viewportRect.bottom - 1
    );
  });
  if (!anchor) {
    return null;
  }
  return {
    element: anchor,
    viewportTop: anchor.getBoundingClientRect().top,
  };
}

function syncTailThinkingScrollReserve(viewport: HTMLElement): boolean {
  const content = transcriptContent(viewport);
  const tailRow = content?.querySelector<HTMLElement>(TAIL_THINKING_ROW_SELECTOR);
  let reserve = 0;
  if (content && tailRow) {
    const rowGap = Number.parseFloat(getComputedStyle(content).rowGap || "0");
    let previous = tailRow.previousElementSibling;
    while (previous instanceof HTMLElement && previous.hidden) {
      previous = previous.previousElementSibling;
    }
    reserve = tailThinkingScrollReserve(
      tailRow.getBoundingClientRect().height,
      rowGap,
      previous instanceof HTMLElement,
    );
  }

  const nextValue = reserve > 0 ? `${reserve}px` : "0px";
  if (
    viewport.style.getPropertyValue(TAIL_THINKING_RESERVE_PROPERTY) ===
    nextValue
  ) {
    return false;
  }
  viewport.style.setProperty(TAIL_THINKING_RESERVE_PROPERTY, nextValue);
  return true;
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

type TailThinkingLayoutSnapshot = {
  anchor: TranscriptScrollAnchorSnapshot | null;
  scopeKey: string | null;
  showTailThinking: boolean;
};

type TranscriptScrollCoordinatorProps = {
  activeMessages: UiTranscriptMessage[];
  activeThreadMessageKey: string | null;
  historyLoading: boolean;
  messagesRef: React.RefObject<HTMLDivElement | null>;
  scopeKey: string | null;
  showTailThinking: boolean;
  scrollIntent: TranscriptScrollIntent | null;
};

/**
 * Single owner for every transcript viewport mutation that must coordinate
 * with the in-flow tail row. Force-bottom and tail-anchor preservation are one
 * explicit transaction, so a tail lifecycle effect (including its repeated
 * ResizeObserver pass) cannot roll back a composer dispatch snap.
 */
export function TranscriptScrollCoordinator({
  activeMessages,
  activeThreadMessageKey,
  historyLoading,
  messagesRef,
  scopeKey,
  showTailThinking,
  scrollIntent,
}: TranscriptScrollCoordinatorProps): null {
  const { scrollToEnd } = useMessageScroller();
  const scrollToEndRef = useRef(scrollToEnd);
  scrollToEndRef.current = scrollToEnd;
  const currentStateRef = useRef({ scopeKey, showTailThinking });
  currentStateRef.current = { scopeKey, showTailThinking };
  const stableLayoutRef = useRef<TailThinkingLayoutSnapshot | null>(null);
  const transactionRef = useRef<TranscriptScrollTransaction | null>(null);
  const transactionRevisionRef = useRef(0);
  const settlementFrameRef = useRef<number | null>(null);

  const captureStableLayout = (viewport: HTMLElement) => {
    const current = currentStateRef.current;
    stableLayoutRef.current = {
      anchor: captureTranscriptScrollAnchor(viewport),
      scopeKey: current.scopeKey,
      showTailThinking: current.showTailThinking,
    };
  };

  const followBottom = () => {
    scrollToEndRef.current({ behavior: "auto" });
  };

  const scheduleTransactionSettlement = (
    viewport: HTMLElement,
    transaction: TranscriptScrollTransaction,
  ) => {
    if (settlementFrameRef.current !== null) {
      window.cancelAnimationFrame(settlementFrameRef.current);
    }
    settlementFrameRef.current = window.requestAnimationFrame(() => {
      settlementFrameRef.current = null;
      if (
        messagesRef.current !== viewport ||
        transactionRef.current?.revision !== transaction.revision
      ) {
        return;
      }
      // ResizeObserver normally settles first. This frame-boundary pass is the
      // deterministic fallback when the browser does not deliver a resize.
      applyTranscriptScrollTransaction(viewport, transaction, followBottom);
      transactionRef.current = settleTranscriptScrollTransaction(
        transactionRef.current,
        transaction,
      );
      captureStableLayout(viewport);
    });
  };

  useLayoutEffect(() => {
    const viewport = messagesRef.current;
    if (!viewport) {
      stableLayoutRef.current = null;
      transactionRef.current = null;
      return;
    }

    const previousLayout = stableLayoutRef.current;
    const tailLifecycleChanged = Boolean(
      previousLayout &&
        previousLayout.scopeKey === scopeKey &&
        previousLayout.showTailThinking !== showTailThinking,
    );
    const intentRefs = scrollIntent;
    const decision = intentRefs
      ? decideTranscriptBottomScroll({
          activeMessages,
          currentThreadId: activeThreadMessageKey,
          forceBottomSnap:
            intentRefs.forceMessagesBottomSnapRef.current,
          historyLoading,
          pendingThreadBottomSnap:
            intentRefs.pendingThreadBottomSnapRef.current,
          previousTailSignature:
            intentRefs.lastRenderedMessageTailSignatureRef.current,
          previousThreadId:
            intentRefs.lastRenderedMessageThreadRef.current,
          shouldStickToBottom:
            intentRefs.shouldStickMessagesToBottomRef.current,
        })
      : null;
    const prependAnchor =
      intentRefs?.pendingMessagesPrependAnchorRef.current ?? null;
    const forceBottom = Boolean(
      !prependAnchor && decision?.forceSnap && decision.shouldSnapToBottom,
    );
    if (tailLifecycleChanged || forceBottom) {
      transactionRevisionRef.current += 1;
    }
    transactionRef.current = beginTranscriptScrollTransaction({
      active: prependAnchor ? null : transactionRef.current,
      anchor: previousLayout?.anchor ?? null,
      forceBottom,
      preserveTailAnchor: tailLifecycleChanged,
      revision: transactionRevisionRef.current,
      scopeKey,
    });

    syncTailThinkingScrollReserve(viewport);

    if (prependAnchor) {
      if (intentRefs && prependAnchor.threadId === activeThreadMessageKey) {
        intentRefs.shouldStickMessagesToBottomRef.current = false;
      }
      if (intentRefs && decision) {
        intentRefs.pendingMessagesPrependAnchorRef.current = null;
        intentRefs.lastRenderedMessageThreadRef.current = activeThreadMessageKey;
        intentRefs.lastRenderedMessageCountRef.current = decision.currentCount;
        intentRefs.lastRenderedMessageTailSignatureRef.current =
          decision.currentTailSignature;
      }
      const transaction = transactionRef.current;
      if (transaction) {
        applyTranscriptScrollTransaction(viewport, transaction, followBottom);
        scheduleTransactionSettlement(viewport, transaction);
      }
      captureStableLayout(viewport);
      return;
    }

    const transaction = transactionRef.current;
    if (transaction?.mode === "force-bottom") {
      applyTranscriptScrollTransaction(viewport, transaction, followBottom);
    } else if (decision?.shouldSnapToBottom) {
      followBottom();
    } else if (decision?.shouldFollowMessageTail) {
      followBottom();
    }
    if (transaction?.mode === "preserve-tail-anchor") {
      applyTranscriptScrollTransaction(viewport, transaction, followBottom);
    }

    if (intentRefs && decision) {
      if (decision.shouldSnapToBottom) {
        intentRefs.pendingThreadBottomSnapRef.current = null;
        intentRefs.forceMessagesBottomSnapRef.current = false;
        if (decision.threadChanged || decision.forceSnap) {
          intentRefs.shouldStickMessagesToBottomRef.current = true;
        }
      } else if (
        decision.pendingSnapMatches &&
        decision.currentCount > 0 &&
        !historyLoading
      ) {
        intentRefs.pendingThreadBottomSnapRef.current = null;
        intentRefs.forceMessagesBottomSnapRef.current = false;
      }
      intentRefs.lastRenderedMessageThreadRef.current =
        activeThreadMessageKey;
      intentRefs.lastRenderedMessageCountRef.current = decision.currentCount;
      intentRefs.lastRenderedMessageTailSignatureRef.current =
        decision.currentTailSignature;
    }

    if (transaction) {
      scheduleTransactionSettlement(viewport, transaction);
    } else if (settlementFrameRef.current !== null) {
      window.cancelAnimationFrame(settlementFrameRef.current);
      settlementFrameRef.current = null;
    }
    captureStableLayout(viewport);
  }, [
    activeThreadMessageKey,
    activeMessages,
    historyLoading,
    messagesRef,
    scopeKey,
    scrollIntent,
    showTailThinking,
  ]);

  useEffect(() => {
    const viewport = messagesRef.current;
    const content = viewport ? transcriptContent(viewport) : null;
    if (!viewport || !content) {
      return;
    }

    let disposed = false;
    const observer =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(() => {
            if (disposed) {
              return;
            }
            const stableBeforeResize =
              stableLayoutRef.current?.anchor ?? null;
            const reserveChanged =
              syncTailThinkingScrollReserve(viewport);
            let transaction = transactionRef.current;
            if (!transaction && reserveChanged) {
              transactionRevisionRef.current += 1;
              transaction = beginTranscriptScrollTransaction({
                active: null,
                anchor: stableBeforeResize,
                forceBottom: false,
                preserveTailAnchor: true,
                revision: transactionRevisionRef.current,
                scopeKey: currentStateRef.current.scopeKey,
              });
              transactionRef.current = transaction;
            }
            if (!transaction) {
              captureStableLayout(viewport);
              return;
            }

            applyTranscriptScrollTransaction(
              viewport,
              transaction,
              followBottom,
            );
            queueMicrotask(() => {
              // Every ResizeObserver shares one delivery round. Re-apply the
              // same transaction after MessageScroller's observer, then settle.
              if (
                disposed ||
                transactionRef.current?.revision !== transaction.revision
              ) {
                return;
              }
              applyTranscriptScrollTransaction(
                viewport,
                transaction,
                followBottom,
              );
              transactionRef.current = settleTranscriptScrollTransaction(
                transactionRef.current,
                transaction,
              );
              if (settlementFrameRef.current !== null) {
                window.cancelAnimationFrame(
                  settlementFrameRef.current,
                );
                settlementFrameRef.current = null;
              }
              captureStableLayout(viewport);
            });
          });
    observer?.observe(content);

    const handleScroll = () => {
      if (!transactionRef.current) {
        captureStableLayout(viewport);
      }
    };
    viewport.addEventListener("scroll", handleScroll, { passive: true });
    captureStableLayout(viewport);

    return () => {
      disposed = true;
      observer?.disconnect();
      viewport.removeEventListener("scroll", handleScroll);
      viewport.style.removeProperty(TAIL_THINKING_RESERVE_PROPERTY);
      if (settlementFrameRef.current !== null) {
        window.cancelAnimationFrame(settlementFrameRef.current);
        settlementFrameRef.current = null;
      }
      transactionRef.current = null;
      stableLayoutRef.current = null;
    };
  }, [messagesRef]);

  return null;
}
