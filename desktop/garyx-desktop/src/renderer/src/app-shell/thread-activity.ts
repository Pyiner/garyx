import type {
  TranscriptMessage,
} from "@shared/contracts";

import type { UiTranscriptMessage } from "./types";

function isLoopContinuationActivityMessage(
  message: Pick<TranscriptMessage, "internal" | "internalKind">,
): boolean {
  return (
    Boolean(message.internal) && message.internalKind === "loop_continuation"
  );
}

function isAssistantProgressRole(role: TranscriptMessage["role"]): boolean {
  return role === "assistant" || role === "tool_use" || role === "tool_result";
}

export function latestUserMessageAwaitsAssistant(
  messages: Array<Pick<UiTranscriptMessage, "role" | "internal" | "internalKind">>,
): boolean {
  let latestUserIndex = -1;
  let latestAssistantOrToolIndex = -1;
  messages.forEach((message, index) => {
    if (message.role === "user" && !isLoopContinuationActivityMessage(message)) {
      latestUserIndex = index;
    }
    if (isAssistantProgressRole(message.role)) {
      latestAssistantOrToolIndex = index;
    }
  });
  return latestUserIndex >= 0 && latestAssistantOrToolIndex < latestUserIndex;
}

export type ThreadActivityModel = {
  runActive: boolean;
  canSteerQueuedPrompt: boolean;
  showPendingAckLoading: boolean;
};

// Cross-platform conversation-state contract (spec/conversation-state +
// iOS conformance twin). This drives only non-render business gates: composer
// lock, steer affordance, and optimistic pre-ack loading. Rendered rows,
// thinking, and tool activity come from the server `render_state`.
export function deriveThreadActivityModel(input: {
  messages: UiTranscriptMessage[];
  runtimeBusy: boolean;
  pendingAckIntentCount: number;
  remoteAwaitingAckInputCount: number;
  pendingHistoryIntent: boolean;
}): ThreadActivityModel {
  const latestUserAwaitsAssistant = latestUserMessageAwaitsAssistant(input.messages);
  const showPendingAckLoading = Boolean(
    input.pendingAckIntentCount > 0 ||
      input.remoteAwaitingAckInputCount > 0 ||
      (input.pendingHistoryIntent && latestUserAwaitsAssistant),
  );
  const runActive = Boolean(input.runtimeBusy);
  return {
    runActive,
    showPendingAckLoading,
    canSteerQueuedPrompt: showPendingAckLoading || runActive,
  };
}
