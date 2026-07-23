import type { UiTranscriptMessage } from "../types.ts";
import {
  restoreTranscriptScrollAnchor,
  type TranscriptScrollAnchorSnapshot,
} from "./transcript-scroll-anchor.ts";

export function messageTailSignature(
  messages: UiTranscriptMessage[],
): string {
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

export type TranscriptBottomScrollDecision = {
  currentCount: number;
  currentTailSignature: string;
  forceSnap: boolean;
  messageTailChanged: boolean;
  pendingSnapMatches: boolean;
  shouldFollowMessageTail: boolean;
  shouldSnapToBottom: boolean;
  threadChanged: boolean;
};

/**
 * Resolve the shell's scroll intent without touching the DOM. The coordinator
 * consumes this decision and owns every resulting viewport mutation.
 */
export function decideTranscriptBottomScroll({
  activeMessages,
  currentThreadId,
  forceBottomSnap,
  historyLoading,
  pendingThreadBottomSnap,
  previousTailSignature,
  previousThreadId,
  shouldStickToBottom,
}: {
  activeMessages: UiTranscriptMessage[];
  currentThreadId: string | null;
  forceBottomSnap: boolean;
  historyLoading: boolean;
  pendingThreadBottomSnap: string | null;
  previousTailSignature: string;
  previousThreadId: string | null;
  shouldStickToBottom: boolean;
}): TranscriptBottomScrollDecision {
  const currentCount = activeMessages.length;
  const currentTailSignature = messageTailSignature(activeMessages);
  const threadChanged = currentThreadId !== previousThreadId;
  const messageTailChanged =
    currentTailSignature !== previousTailSignature;
  const pendingSnapMatches =
    pendingThreadBottomSnap === currentThreadId;
  const forceSnap = pendingSnapMatches && forceBottomSnap;
  const canScrollCurrentThread = Boolean(
    currentThreadId && currentCount > 0 && !historyLoading,
  );

  return {
    currentCount,
    currentTailSignature,
    forceSnap,
    messageTailChanged,
    pendingSnapMatches,
    shouldFollowMessageTail: Boolean(
      currentThreadId &&
        !historyLoading &&
        messageTailChanged &&
        shouldStickToBottom,
    ),
    shouldSnapToBottom: Boolean(
      canScrollCurrentThread &&
        (threadChanged ||
          forceSnap ||
          (pendingSnapMatches && shouldStickToBottom)),
    ),
    threadChanged,
  };
}

export type TranscriptScrollTransaction =
  | {
      readonly anchor: TranscriptScrollAnchorSnapshot | null;
      readonly mode: "preserve-tail-anchor";
      readonly revision: number;
      readonly scopeKey: string | null;
    }
  | {
      readonly mode: "force-bottom";
      readonly revision: number;
      readonly scopeKey: string | null;
    };

/**
 * Begin the one scroll transaction for this commit. A force-bottom request is
 * authoritative: it replaces a pending tail-anchor transaction and cannot be
 * downgraded by a later tail/ResizeObserver pass before it settles.
 */
export function beginTranscriptScrollTransaction({
  active,
  anchor,
  forceBottom,
  preserveTailAnchor,
  revision,
  scopeKey,
}: {
  active: TranscriptScrollTransaction | null;
  anchor: TranscriptScrollAnchorSnapshot | null;
  forceBottom: boolean;
  preserveTailAnchor: boolean;
  revision: number;
  scopeKey: string | null;
}): TranscriptScrollTransaction | null {
  const activeInScope = active?.scopeKey === scopeKey ? active : null;
  if (forceBottom) {
    return {
      mode: "force-bottom",
      revision,
      scopeKey,
    };
  }
  if (activeInScope?.mode === "force-bottom") {
    return activeInScope;
  }
  if (preserveTailAnchor) {
    return {
      anchor,
      mode: "preserve-tail-anchor",
      revision,
      scopeKey,
    };
  }
  return activeInScope;
}

/**
 * Apply the active transaction. Re-applying is intentional: ResizeObserver
 * delivery may run after the initial layout pass, so the same authoritative
 * action must win both times.
 */
export function applyTranscriptScrollTransaction(
  viewport: Pick<HTMLElement, "contains" | "scrollTop">,
  transaction: TranscriptScrollTransaction,
  followBottom: () => void,
): number {
  const previousScrollTop = viewport.scrollTop;
  if (transaction.mode === "force-bottom") {
    followBottom();
  } else {
    restoreTranscriptScrollAnchor(viewport, transaction.anchor);
  }
  return viewport.scrollTop - previousScrollTop;
}

export function settleTranscriptScrollTransaction(
  active: TranscriptScrollTransaction | null,
  completed: TranscriptScrollTransaction,
): TranscriptScrollTransaction | null {
  return active?.revision === completed.revision ? null : active;
}
