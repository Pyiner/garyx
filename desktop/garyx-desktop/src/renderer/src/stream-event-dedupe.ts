import type { DesktopChatStreamEvent } from '@shared/contracts';

const MAX_STREAM_EVENT_DEDUPE_KEYS = 2000;

export type ChatStreamEventDedupeState = {
  seenKeys: Map<string, true>;
};

export function createChatStreamEventDedupeState(): ChatStreamEventDedupeState {
  return {
    seenKeys: new Map(),
  };
}

function streamEventDedupeKey(event: DesktopChatStreamEvent): string | null {
  if (
    event.eventSeq === undefined ||
    !Number.isSafeInteger(event.eventSeq) ||
    event.eventSeq < 0
  ) {
    return null;
  }
  return `${event.threadId}\u0000${event.runId}\u0000${event.eventSeq}`;
}

export function shouldAcceptChatStreamEvent(
  state: ChatStreamEventDedupeState,
  event: DesktopChatStreamEvent,
): boolean {
  const key = streamEventDedupeKey(event);
  if (!key) {
    return true;
  }
  if (state.seenKeys.has(key)) {
    return false;
  }
  state.seenKeys.set(key, true);
  while (state.seenKeys.size > MAX_STREAM_EVENT_DEDUPE_KEYS) {
    const oldest = state.seenKeys.keys().next().value;
    if (oldest === undefined) {
      break;
    }
    state.seenKeys.delete(oldest);
  }
  return true;
}
