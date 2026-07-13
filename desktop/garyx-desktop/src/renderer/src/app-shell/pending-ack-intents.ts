import type { TranscriptMessage } from "@shared/contracts";

import type { MessageIntent } from "../message-machine.ts";
import { messageOriginId } from "../gateway-mirror/transcript-materialize.ts";

/**
 * Build the exact two ids accepted by the legacy pending-ack matcher once per
 * message array: the optimistic intent id and the normalized server origin id.
 */
export function representedUserIntentIds(
  messages: readonly (TranscriptMessage & { intentId?: string })[],
): Set<string> {
  const ids = new Set<string>();
  for (const message of messages) {
    if (message.role !== "user") {
      continue;
    }
    if (message.intentId) {
      ids.add(message.intentId);
    }
    const originId = messageOriginId(message);
    if (originId) {
      ids.add(originId);
    }
  }
  return ids;
}

export function pendingAckIntentsNotRepresented(
  intents: readonly MessageIntent[],
  representedIntentIds: ReadonlySet<string>,
): MessageIntent[] {
  return intents.filter(
    (intent) => !representedIntentIds.has(intent.intentId),
  );
}
