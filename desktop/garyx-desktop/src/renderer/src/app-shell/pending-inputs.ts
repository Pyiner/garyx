import type {
  PendingThreadInput,
  TranscriptMessage,
} from "@shared/contracts";

function metadataString(
  metadata: Record<string, unknown> | null | undefined,
  key: string,
): string {
  const value = metadata?.[key];
  return typeof value === "string" ? value.trim() : "";
}

function userMessageRepresentsPendingInput(
  message: TranscriptMessage,
  pendingInput: PendingThreadInput,
): boolean {
  if (message.role !== "user") {
    return false;
  }
  const pendingInputId = pendingInput.id.trim();
  if (!pendingInputId) {
    return false;
  }
  return metadataString(message.metadata, "queued_input_id") === pendingInputId;
}

export function visibleRemotePendingInputsForThread(input: {
  activeMessages: readonly TranscriptMessage[];
  visiblePendingAckIntentCount: number;
  remotePendingInputs: readonly PendingThreadInput[];
}): PendingThreadInput[] {
  if (input.visiblePendingAckIntentCount > 0) {
    return [];
  }
  return input.remotePendingInputs.filter((pending) => {
    return (
      pending.status === "awaiting_ack" &&
      !input.activeMessages.some((message) =>
        userMessageRepresentsPendingInput(message, pending),
      )
    );
  });
}
