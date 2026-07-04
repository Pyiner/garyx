// Pure transcript materialization/reconciliation helpers, moved out of
// useTranscriptController (endgame batch 2a-1). These are React-free and
// shared by the legacy hook (via re-export) and the GatewayMirror
// transcript domain. No logic changes: verbatim relocation.

import type {
  DesktopChatStreamEvent,
  TranscriptMessage,
} from "@shared/contracts";
import {
  isControlTranscriptMessage,
  isToolRole,
  transcriptControlKind,
} from "../../../shared/transcript-sync.ts";
import {
  type MessageIntent,
} from "../message-machine.ts";
import {
  countTranscriptFiles,
  countTranscriptImages,
  extractTranscriptText,
} from "../message-rich-content-core.ts";
import {
  extractImageGenerationImageContent,
} from "../app-shell/image-generation-content.ts";
import {
  isRunLoadingPlaceholderMessage,
} from "../app-shell/loading-labels.ts";
import type {
  UiTranscriptMessage,
} from "../app-shell/types.ts";

const MESSAGES_TOP_PAGINATION_PREFETCH_MIN_PX = 640;
const MESSAGES_TOP_PAGINATION_PREFETCH_VIEWPORTS = 1.5;
const USER_TURN_PREFETCH_THRESHOLD = 3;

export function messagesNearEarlierUserTurnBoundary(
  node: HTMLDivElement | null,
): boolean {
  if (!node) {
    return false;
  }
  const pixelPrefetchDistance = Math.max(
    MESSAGES_TOP_PAGINATION_PREFETCH_MIN_PX,
    node.clientHeight * MESSAGES_TOP_PAGINATION_PREFETCH_VIEWPORTS,
  );
  if (node.scrollTop <= pixelPrefetchDistance) {
    return true;
  }
  const viewportTop = node.getBoundingClientRect().top;
  const userTurnStarts = node.querySelectorAll<HTMLElement>(
    "[data-user-turn-start='true']",
  );
  if (userTurnStarts.length === 0) {
    return false;
  }
  let userTurnsBeforeViewport = 0;
  for (const turnStart of userTurnStarts) {
    if (turnStart.getBoundingClientRect().bottom <= viewportTop) {
      userTurnsBeforeViewport += 1;
      continue;
    }
    break;
  }
  return userTurnsBeforeViewport <= USER_TURN_PREFETCH_THRESHOLD;
}

export function transcriptEntryHistoryIndex(
  message: Pick<UiTranscriptMessage, "id" | "localState" | "seq">,
): number | null {
  if (message.localState !== "remote_final") {
    return null;
  }
  if (typeof message.seq === "number" && Number.isFinite(message.seq)) {
    return Math.max(0, message.seq - 1);
  }
  const suffix = message.id.split(":").pop();
  if (!suffix || !/^\d+$/.test(suffix)) {
    return null;
  }
  return Number(suffix);
}

export function earliestRemoteHistoryIndex(messages: UiTranscriptMessage[]): number | null {
  let earliest: number | null = null;
  for (const message of messages) {
    const historyIndex = transcriptEntryHistoryIndex(message);
    if (historyIndex === null) {
      continue;
    }
    if (earliest === null || historyIndex < earliest) {
      earliest = historyIndex;
    }
  }
  return earliest;
}

export function transcriptHasAutomationResponse(
  messages: TranscriptMessage[],
): boolean {
  return visibleTranscriptMessages(messages).some(
    (message) => message.role === "assistant" || isToolRole(message.role),
  );
}

export function visibleTranscriptMessages(
  messages: TranscriptMessage[],
): TranscriptMessage[] {
  return messages.filter((message) => !isControlTranscriptMessage(message));
}

export function normalizeMessageText(value: string | undefined): string {
  return value?.trim() || "";
}

export function transcriptMessageImageCount(message: TranscriptMessage): number {
  return countTranscriptImages(message.content);
}

export function transcriptMessageFileCount(message: TranscriptMessage): number {
  return countTranscriptFiles(message.content);
}

export function transcriptMessageComparableText(message: TranscriptMessage): string {
  const structuredText = normalizeMessageText(
    extractTranscriptText(message.content),
  );
  if (structuredText) {
    return structuredText;
  }
  if (
    transcriptMessageImageCount(message) > 0 ||
    transcriptMessageFileCount(message) > 0
  ) {
    return "";
  }
  return normalizeMessageText(message.text);
}

export function uiTranscriptMessageComparableText(
  message: UiTranscriptMessage,
): string {
  const structuredText = normalizeMessageText(
    extractTranscriptText(message.content),
  );
  if (structuredText) {
    return structuredText;
  }
  if (
    transcriptMessageImageCount(message) > 0 ||
    transcriptMessageFileCount(message) > 0
  ) {
    return "";
  }
  return normalizeMessageText(message.text);
}

export function isRecoverableAssistantEntry(
  entry: UiTranscriptMessage,
  intentId: string,
  candidateEntryIds: Set<string>,
): boolean {
  if (entry.role !== "assistant" || entry.intentId !== intentId) {
    return false;
  }
  return (
    entry.pending ||
    entry.localState === "optimistic" ||
    entry.localState === "remote_partial" ||
    candidateEntryIds.has(entry.id)
  );
}

export function reconcileAssistantEntriesForGatewayRecovery(
  entries: UiTranscriptMessage[],
  intentId: string,
  candidateEntryIds: Iterable<string | null | undefined>,
): { entries: UiTranscriptMessage[]; matched: boolean } {
  const normalizedCandidateEntryIds = new Set(
    [...candidateEntryIds]
      .map((value) => value?.trim() || "")
      .filter((value) => value.length > 0),
  );
  let matched = false;
  const nextEntries: UiTranscriptMessage[] = [];

  for (const entry of entries) {
    if (
      !isRecoverableAssistantEntry(entry, intentId, normalizedCandidateEntryIds)
    ) {
      nextEntries.push(entry);
      continue;
    }

    matched = true;
    const visibleText = uiTranscriptMessageComparableText(entry);
    if (!visibleText) {
      continue;
    }

    nextEntries.push({
      ...entry,
      pending: false,
      error: false,
      localState:
        entry.localState === "optimistic" ? "remote_partial" : entry.localState,
    });
  }

  return {
    entries: nextEntries,
    matched,
  };
}

export function transcriptMessageMatchesIntent(
  message: TranscriptMessage,
  intent: MessageIntent,
): boolean {
  return messageOriginId(message) === intent.intentId;
}

export function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  return value as Record<string, unknown>;
}

export function metadataString(
  metadata: Record<string, unknown> | null | undefined,
  key: string,
): string {
  const value = metadata?.[key];
  return typeof value === "string" ? value.trim() : "";
}

export function userMessageIdForOrigin(originId: string): string {
  return `origin:${originId}`;
}

export function messageOriginId(
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

export function normalizeTranscriptMessageId(
  message: TranscriptMessage,
): TranscriptMessage {
  const originId = messageOriginId(message);
  if (!originId) {
    return message;
  }
  const id = userMessageIdForOrigin(originId);
  return message.id === id ? message : { ...message, id };
}

const GENERATED_IMAGE_TOOL_USE_METADATA_KEY = "generated_image_tool_use_id";

export function jsonValuesEqual(left: unknown, right: unknown): boolean {
  return JSON.stringify(left ?? null) === JSON.stringify(right ?? null);
}

export function remoteTranscriptMessageCanReuseExisting(
  existing: UiTranscriptMessage,
  remote: TranscriptMessage,
  options?: { ignoreTimestamp?: boolean },
): boolean {
  return (
    existing.localState === "remote_final" &&
    existing.role === remote.role &&
    existing.text === remote.text &&
    jsonValuesEqual(existing.content, remote.content) &&
    (options?.ignoreTimestamp || existing.timestamp === remote.timestamp) &&
    existing.toolUseId === remote.toolUseId &&
    existing.toolName === remote.toolName &&
    existing.isError === remote.isError &&
    jsonValuesEqual(existing.metadata, remote.metadata) &&
    existing.kind === remote.kind &&
    existing.internal === remote.internal &&
    existing.internalKind === remote.internalKind &&
    existing.loopOrigin === remote.loopOrigin &&
    existing.pending !== true &&
    existing.error === remote.error
  );
}

export function materializeRemoteTranscript(
  transcript: TranscriptMessage[],
  existing: UiTranscriptMessage[],
  options?: { ignoreTimestampForStableMessages?: boolean },
): UiTranscriptMessage[] {
  const usedExistingIndexes = new Set<number>();

  const materializeMessage = (
    message: TranscriptMessage,
  ): UiTranscriptMessage => {
    let matchedIndex = existing.findIndex((entry, index) => {
      return !usedExistingIndexes.has(index) && entry.id === message.id;
    });

    const matchedEntry = matchedIndex >= 0 ? existing[matchedIndex] : null;
    if (matchedIndex >= 0) {
      usedExistingIndexes.add(matchedIndex);
    }

    if (
      matchedEntry &&
      remoteTranscriptMessageCanReuseExisting(matchedEntry, message, {
        ignoreTimestamp: options?.ignoreTimestampForStableMessages,
      })
    ) {
      // Keep the stable id for React, but carry the committed seq so render_state
      // refs can resolve this body (the reused entry may be an optimistic one
      // that never had a seq).
      return matchedEntry.seq === message.seq
        ? matchedEntry
        : { ...matchedEntry, seq: message.seq ?? matchedEntry.seq };
    }

    return {
      ...message,
      id: matchedEntry?.id || message.id,
      intentId: matchedEntry?.intentId,
      remoteRunId: matchedEntry?.remoteRunId,
      localState: "remote_final" as const,
      pending: false,
      error: message.error,
    };
  };

  const materializeGeneratedImageMessage = (
    sourceMessage: TranscriptMessage,
    content: unknown[],
  ): UiTranscriptMessage => {
    const toolUseId = sourceMessage.toolUseId?.trim() || "";
    const synthetic: TranscriptMessage = {
      id: `generated-image:${sourceMessage.id}`,
      role: "assistant",
      text: "",
      content,
      timestamp: sourceMessage.timestamp,
      metadata: {
        source: "codex_app_server",
        item_type: "imageGeneration",
        [GENERATED_IMAGE_TOOL_USE_METADATA_KEY]: toolUseId,
      },
      kind: "assistant_reply",
    };
    let matchedIndex = existing.findIndex((entry, index) => {
      return !usedExistingIndexes.has(index) && entry.id === synthetic.id;
    });
    if (matchedIndex < 0 && toolUseId) {
      matchedIndex = existing.findIndex((entry, index) => {
        const metadata = asRecord(entry.metadata);
        return (
          !usedExistingIndexes.has(index) &&
          entry.role === "assistant" &&
          metadata?.[GENERATED_IMAGE_TOOL_USE_METADATA_KEY] === toolUseId
        );
      });
    }
    if (matchedIndex < 0) {
      const contentSignature = JSON.stringify(content);
      matchedIndex = existing.findIndex((entry, index) => {
        return (
          !usedExistingIndexes.has(index) &&
          entry.role === "assistant" &&
          !entry.text.trim() &&
          JSON.stringify(entry.content) === contentSignature
        );
      });
    }

    const matchedEntry = matchedIndex >= 0 ? existing[matchedIndex] : null;
    if (matchedIndex >= 0) {
      usedExistingIndexes.add(matchedIndex);
    }

    if (
      matchedEntry &&
      remoteTranscriptMessageCanReuseExisting(matchedEntry, synthetic, {
        ignoreTimestamp: options?.ignoreTimestampForStableMessages,
      })
    ) {
      return matchedEntry;
    }

    return {
      ...synthetic,
      id: matchedEntry?.id || synthetic.id,
      intentId: matchedEntry?.intentId,
      remoteRunId: matchedEntry?.remoteRunId,
      localState: "remote_final" as const,
      pending: false,
      error: false,
    };
  };

  const materializedRemote: UiTranscriptMessage[] = [];
  for (const message of transcript) {
    if (isControlTranscriptMessage(message)) {
      continue;
    }
    if (isRunLoadingPlaceholderMessage(message)) {
      continue;
    }
    const normalizedMessage = normalizeTranscriptMessageId(message);
    materializedRemote.push(materializeMessage(normalizedMessage));
    if (message.role === "tool_result") {
      const imageContent = extractImageGenerationImageContent(message);
      if (imageContent) {
        materializedRemote.push(
          materializeGeneratedImageMessage(message, imageContent),
        );
      }
    }
  }
  return materializedRemote;
}

export function resolveIntentHistoryMatch(
  intent: MessageIntent,
  messages: TranscriptMessage[],
) {
  const userIndex =
    [...messages]
      .map((message, index) => ({ message, index }))
      .reverse()
      .find(({ message }) => {
        return transcriptMessageMatchesIntent(message, intent);
      })?.index ?? -1;

  if (userIndex < 0) {
    return {
      userVisible: false,
      assistantVisible: false,
    };
  }

  const followUpMessages = messages.slice(userIndex + 1);
  const assistantMessages = followUpMessages.filter(
    (message) => message.role === "assistant",
  );
  const expectedResponse = normalizeMessageText(intent.responseText);
  const assistantVisible = expectedResponse
    ? assistantMessages.some(
        (message) => normalizeMessageText(message.text) === expectedResponse,
      )
    : assistantMessages.length > 0 ||
      followUpMessages.some((message) => isToolRole(message.role));

  return {
    userVisible: true,
    assistantVisible,
  };
}

export function chatStreamEventHasRunLifecycle(event: DesktopChatStreamEvent): boolean {
  const events =
    event.type === "thread_render_frame"
      ? event.events
      : event.type === "committed_message"
        ? [event]
        : [];
  return events.some((committed) => {
    const controlKind = transcriptControlKind(committed.message);
    return (
      controlKind === "run_start" ||
      controlKind === "run_complete" ||
      controlKind === "run_interrupted" ||
      controlKind === "interrupt_confirmed"
    );
  });
}

