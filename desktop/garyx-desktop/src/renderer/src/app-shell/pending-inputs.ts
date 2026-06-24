import type {
  PendingThreadInput,
  TranscriptMessage,
} from "@shared/contracts";

export type PendingInputOriginRef = {
  pendingInputId?: string | null;
  originId?: string | null;
};

function metadataString(
  metadata: Record<string, unknown> | null | undefined,
  key: string,
): string {
  const value = metadata?.[key];
  return typeof value === "string" ? value.trim() : "";
}

function normalizedString(value: string | null | undefined): string {
  return value?.trim() || "";
}

function messageOriginId(
  message: Pick<TranscriptMessage, "id" | "metadata" | "role">,
): string {
  if (message.role !== "user") {
    return "";
  }
  if (message.id.startsWith("origin:")) {
    return message.id.slice("origin:".length).trim();
  }
  return metadataString(message.metadata, "origin_id");
}

function pendingInputOriginRefsByPendingInputId(
  refs: readonly PendingInputOriginRef[] | undefined,
): Map<string, Set<string>> {
  const refsByPendingInputId = new Map<string, Set<string>>();
  for (const ref of refs || []) {
    const pendingInputId = normalizedString(ref.pendingInputId);
    const originId = normalizedString(ref.originId);
    if (!pendingInputId || !originId) {
      continue;
    }
    const originIds =
      refsByPendingInputId.get(pendingInputId) ?? new Set<string>();
    originIds.add(originId);
    refsByPendingInputId.set(pendingInputId, originIds);
  }
  return refsByPendingInputId;
}

function userMessageRepresentsPendingInput(
  message: TranscriptMessage,
  pendingInput: PendingThreadInput,
  originRefsByPendingInputId: Map<string, Set<string>>,
): boolean {
  if (message.role !== "user") {
    return false;
  }
  const pendingInputId = pendingInput.id.trim();
  if (!pendingInputId) {
    return false;
  }
  if (metadataString(message.metadata, "queued_input_id") === pendingInputId) {
    return true;
  }
  const originIds = originRefsByPendingInputId.get(pendingInputId);
  if (!originIds?.size) {
    return false;
  }
  const originId = messageOriginId(message);
  return Boolean(originId && originIds.has(originId));
}

export function visibleRemotePendingInputsForThread(input: {
  activeMessages: readonly TranscriptMessage[];
  visiblePendingAckIntentCount: number;
  remotePendingInputs: readonly PendingThreadInput[];
  pendingInputOriginRefs?: readonly PendingInputOriginRef[];
}): PendingThreadInput[] {
  if (input.visiblePendingAckIntentCount > 0) {
    return [];
  }
  const originRefsByPendingInputId = pendingInputOriginRefsByPendingInputId(
    input.pendingInputOriginRefs,
  );
  return input.remotePendingInputs.filter((pending) => {
    return (
      pending.status === "awaiting_ack" &&
      !input.activeMessages.some((message) =>
        userMessageRepresentsPendingInput(
          message,
          pending,
          originRefsByPendingInputId,
        ),
      )
    );
  });
}
