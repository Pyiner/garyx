import type {
  PendingThreadInput,
  ThreadRuntimeInfo,
  TranscriptMessage,
} from "@shared/contracts";

import type { LiveStreamState, UiTranscriptMessage } from "./types";

type ActivityMessage = Pick<
  TranscriptMessage,
  "id" | "role" | "text" | "timestamp" | "kind" | "internalKind"
> &
  Partial<Pick<TranscriptMessage, "internal">>;

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

function isLiveStreamActive(liveStream: LiveStreamState | null | undefined): boolean {
  return Boolean(
    liveStream &&
      ["connecting", "streaming", "reconciling"].includes(
        liveStream.streamStatus,
      ),
  );
}

function canSteerLiveStream(liveStream: LiveStreamState | null | undefined): boolean {
  return Boolean(
    liveStream &&
      ["connecting", "streaming"].includes(liveStream.streamStatus),
  );
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

export function activeRunAwaitsAssistant(input: {
  threadInfo: Pick<ThreadRuntimeInfo, "activeRun"> | null | undefined;
  messages: UiTranscriptMessage[];
  suppressForPendingAck?: boolean;
}): boolean {
  return Boolean(
    input.threadInfo?.activeRun?.runId &&
      latestUserMessageAwaitsAssistant(input.messages) &&
      !input.suppressForPendingAck,
  );
}

export type ThreadActivityModel = {
  runActive: boolean;
  canSteerQueuedPrompt: boolean;
  showPendingAckLoading: boolean;
  showRunLoading: boolean;
};

export function deriveThreadActivityModel(input: {
  messages: UiTranscriptMessage[];
  threadInfo: Pick<ThreadRuntimeInfo, "activeRun"> | null | undefined;
  liveStream: LiveStreamState | null | undefined;
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
  const runActive = Boolean(
    isLiveStreamActive(input.liveStream) ||
      input.runtimeBusy ||
      input.threadInfo?.activeRun?.runId,
  );
  const hasPendingAssistant = input.messages.some(
    (message) => message.role === "assistant" && Boolean(message.pending),
  );
  return {
    runActive,
    canSteerQueuedPrompt: canSteerLiveStream(input.liveStream),
    showPendingAckLoading,
    showRunLoading:
      runActive &&
      !showPendingAckLoading &&
      !hasPendingAssistant,
  };
}

export function threadActivitySignature(
  messages: ActivityMessage[],
  pendingInputs: PendingThreadInput[],
  threadInfo?: Pick<ThreadRuntimeInfo, "activeRun"> | null,
): string {
  const lastMessage = messages[messages.length - 1];
  const lastPendingInput = pendingInputs[pendingInputs.length - 1];
  const activeRun = threadInfo?.activeRun || null;
  return JSON.stringify({
    messageCount: messages.length,
    lastMessage: {
      id: lastMessage?.id || "",
      role: lastMessage?.role || "",
      text: lastMessage?.text || "",
      timestamp: lastMessage?.timestamp || "",
      kind: lastMessage?.kind || "",
      internalKind: lastMessage?.internalKind || "",
    },
    pendingInputCount: pendingInputs.length,
    lastPendingInput: {
      id: lastPendingInput?.id || "",
      status: lastPendingInput?.status || "",
      active: Boolean(lastPendingInput?.active),
      text: lastPendingInput?.text || "",
    },
    activeRun: activeRun
      ? {
          runId: activeRun.runId || "",
          updatedAt: activeRun.updatedAt || "",
          assistantResponse: activeRun.assistantResponse || "",
          pendingUserInputCount: activeRun.pendingUserInputCount ?? null,
        }
      : null,
  });
}
