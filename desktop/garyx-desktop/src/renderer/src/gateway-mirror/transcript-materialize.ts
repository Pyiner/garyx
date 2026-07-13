// Pure transcript materialization/reconciliation helpers, moved out of
// useTranscriptController (endgame batch 2a-1). These are React-free and
// shared by the legacy hook (via re-export) and the GatewayMirror
// transcript domain. No logic changes: verbatim relocation.

import type {
  DesktopChatStreamEvent,
  ThreadTranscript,
  TranscriptMessage,
} from "@shared/contracts";
import {
  isControlTranscriptMessage,
  isToolRole,
  mergeForwardTranscriptPage,
  transcriptMessageIndex,
  toolMessagesEquivalent,
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

type JsonComparisonContext = "top" | "array" | "object";

const jsonWireValidity = new WeakMap<object, boolean>();

function jsonValueIsOmitted(value: unknown): boolean {
  const valueType = typeof value;
  return (
    valueType === "undefined" ||
    valueType === "function" ||
    valueType === "symbol"
  );
}

function isJsonWireValue(
  value: unknown,
  context: JsonComparisonContext,
  visiting: Set<object>,
): boolean {
  if (value === null) {
    return true;
  }
  const valueType = typeof value;
  if (
    valueType === "string" ||
    valueType === "boolean" ||
    valueType === "number"
  ) {
    return true;
  }
  if (valueType === "bigint") {
    return false;
  }
  if (
    valueType === "undefined" ||
    valueType === "function" ||
    valueType === "symbol"
  ) {
    return context !== "top";
  }
  if (valueType !== "object") {
    return false;
  }

  const objectValue = value as object;
  const cached = jsonWireValidity.get(objectValue);
  if (cached !== undefined) {
    return cached;
  }
  if (visiting.has(objectValue)) {
    jsonWireValidity.set(objectValue, false);
    return false;
  }
  const prototype = Object.getPrototypeOf(objectValue);
  if (
    !Array.isArray(objectValue) &&
    prototype !== Object.prototype &&
    prototype !== null
  ) {
    jsonWireValidity.set(objectValue, false);
    return false;
  }
  if (
    "toJSON" in (objectValue as Record<string, unknown>) &&
    typeof (objectValue as { toJSON?: unknown }).toJSON === "function"
  ) {
    jsonWireValidity.set(objectValue, false);
    return false;
  }

  visiting.add(objectValue);
  let valid = true;
  if (Array.isArray(objectValue)) {
    for (let index = 0; index < objectValue.length; index += 1) {
      if (!isJsonWireValue(objectValue[index], "array", visiting)) {
        valid = false;
        break;
      }
    }
  } else {
    const record = objectValue as Record<string, unknown>;
    for (const key of Object.keys(record)) {
      if (!isJsonWireValue(record[key], "object", visiting)) {
        valid = false;
        break;
      }
    }
  }
  visiting.delete(objectValue);
  jsonWireValidity.set(objectValue, valid);
  return valid;
}

function jsonComparableValuesEqual(
  left: unknown,
  right: unknown,
  context: JsonComparisonContext,
): boolean {
  if (context === "array") {
    if (jsonValueIsOmitted(left)) {
      left = null;
    }
    if (jsonValueIsOmitted(right)) {
      right = null;
    }
  }
  if (left === right) {
    return true;
  }
  if (
    (left === null ||
      (typeof left === "number" && !Number.isFinite(left))) &&
    (right === null ||
      (typeof right === "number" && !Number.isFinite(right)))
  ) {
    return true;
  }
  if (typeof left !== typeof right) {
    return false;
  }
  if (Array.isArray(left) || Array.isArray(right)) {
    if (!Array.isArray(left) || !Array.isArray(right)) {
      return false;
    }
    if (left.length !== right.length) {
      return false;
    }
    for (let index = 0; index < left.length; index += 1) {
      if (!jsonComparableValuesEqual(left[index], right[index], "array")) {
        return false;
      }
    }
    return true;
  }
  if (
    !left ||
    !right ||
    typeof left !== "object" ||
    typeof right !== "object"
  ) {
    return false;
  }

  const leftRecord = left as Record<string, unknown>;
  const rightRecord = right as Record<string, unknown>;
  const leftKeys = Object.keys(leftRecord);
  const rightKeys = Object.keys(rightRecord);
  let leftIndex = 0;
  let rightIndex = 0;
  while (true) {
    while (
      leftIndex < leftKeys.length &&
      jsonValueIsOmitted(leftRecord[leftKeys[leftIndex]])
    ) {
      leftIndex += 1;
    }
    while (
      rightIndex < rightKeys.length &&
      jsonValueIsOmitted(rightRecord[rightKeys[rightIndex]])
    ) {
      rightIndex += 1;
    }
    if (leftIndex >= leftKeys.length || rightIndex >= rightKeys.length) {
      return leftIndex >= leftKeys.length && rightIndex >= rightKeys.length;
    }
    const leftKey = leftKeys[leftIndex];
    const rightKey = rightKeys[rightIndex];
    if (
      leftKey !== rightKey ||
      !jsonComparableValuesEqual(
        leftRecord[leftKey],
        rightRecord[rightKey],
        "object",
      )
    ) {
      return false;
    }
    leftIndex += 1;
    rightIndex += 1;
  }
}

/**
 * Compare parsed JSON values using the equality relation previously supplied
 * by `JSON.stringify(left ?? null) === JSON.stringify(right ?? null)`, without
 * allocating full serialized strings. Unsupported non-wire values are never
 * reusable.
 */
export function jsonValuesEqual(left: unknown, right: unknown): boolean {
  const normalizedLeft = left ?? null;
  const normalizedRight = right ?? null;
  if (
    !isJsonWireValue(normalizedLeft, "top", new Set()) ||
    !isJsonWireValue(normalizedRight, "top", new Set())
  ) {
    return false;
  }
  return jsonComparableValuesEqual(normalizedLeft, normalizedRight, "top");
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
  type ExistingIndexQueue = { indexes: number[]; cursor: number };
  const indexesById = new Map<string, ExistingIndexQueue>();
  const generatedImageIndexesByToolUseId = new Map<
    string,
    ExistingIndexQueue
  >();
  const generatedImageContentCandidates: number[] = [];

  const appendIndex = (
    index: Map<string, ExistingIndexQueue>,
    key: string,
    existingIndex: number,
  ) => {
    const queue = index.get(key);
    if (queue) {
      queue.indexes.push(existingIndex);
      return;
    }
    index.set(key, { indexes: [existingIndex], cursor: 0 });
  };

  existing.forEach((entry, index) => {
    appendIndex(indexesById, entry.id, index);
    if (entry.role !== "assistant") {
      return;
    }
    const generatedImageToolUseIdValue = asRecord(entry.metadata)?.[
      GENERATED_IMAGE_TOOL_USE_METADATA_KEY
    ];
    const generatedImageToolUseId =
      typeof generatedImageToolUseIdValue === "string"
        ? generatedImageToolUseIdValue
        : "";
    if (generatedImageToolUseId) {
      appendIndex(
        generatedImageIndexesByToolUseId,
        generatedImageToolUseId,
        index,
      );
    }
    if (!entry.text.trim()) {
      generatedImageContentCandidates.push(index);
    }
  });

  const takeFirstUnused = (queue: ExistingIndexQueue | undefined): number => {
    if (!queue) {
      return -1;
    }
    while (
      queue.cursor < queue.indexes.length &&
      usedExistingIndexes.has(queue.indexes[queue.cursor])
    ) {
      queue.cursor += 1;
    }
    if (queue.cursor >= queue.indexes.length) {
      return -1;
    }
    const index = queue.indexes[queue.cursor];
    queue.cursor += 1;
    return index;
  };

  const materializeMessage = (
    message: TranscriptMessage,
  ): UiTranscriptMessage => {
    const matchedIndex = takeFirstUnused(indexesById.get(message.id));

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
    let matchedIndex = takeFirstUnused(indexesById.get(synthetic.id));
    if (matchedIndex < 0 && toolUseId) {
      matchedIndex = takeFirstUnused(
        generatedImageIndexesByToolUseId.get(toolUseId),
      );
    }
    if (matchedIndex < 0) {
      matchedIndex =
        generatedImageContentCandidates.find((index) => {
          return (
            !usedExistingIndexes.has(index) &&
            jsonValuesEqual(existing[index].content, content)
          );
        }) ?? -1;
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

// ---- Batch 2a-2 part 2: remote-transcript apply helpers, moved verbatim ----
// from useTranscriptController. The only signature change: intentForId is an
// injected option instead of a hook closure, so the mirror (which does not
// own the message machine until batch 3) can pass its own lookup.

export const THREAD_HISTORY_PAGE_SIZE = 100;
export const THREAD_HISTORY_USER_QUERY_LIMIT = 10;

/**
 * True when a history response describes a thread the gateway does not
 * know: remoteFound is false and the payload carries no content at all.
 * Shared by ensureThreadOpenable and the selected-thread loader's
 * missing-thread gate (batch 4b) so the predicate cannot drift.
 */
export function isMissingThreadTranscript(
  transcript: ThreadTranscript,
): boolean {
  return (
    !transcript.remoteFound &&
    transcript.messages.length === 0 &&
    transcript.pendingInputs.length === 0 &&
    !transcript.threadInfo
  );
}

export type ThreadHistoryPaginationState = {
  hasMoreBefore: boolean;
  nextBeforeIndex: number | null;
  loadingBefore: boolean;
};

export function paginationStateFromTranscript(
  transcript: ThreadTranscript,
  loadingBefore = false,
): ThreadHistoryPaginationState {
  return {
    hasMoreBefore: Boolean(transcript.pageInfo?.hasMoreBefore),
    nextBeforeIndex:
      typeof transcript.pageInfo?.nextBeforeIndex === "number"
        ? transcript.pageInfo.nextBeforeIndex
        : null,
    loadingBefore,
  };
}

/**
 * Merge an incoming (full/forward-fetch) pagination state onto the current
 * one. Verbatim from applyRemoteTranscript's updateThreadHistoryPagination
 * updater; `existingMessages` is the thread's message cache BEFORE this
 * apply's merge (legacy read messagesByThreadRef at the same point).
 */
export function mergeRemotePaginationState(
  current: ThreadHistoryPaginationState | null,
  incoming: ThreadHistoryPaginationState,
  existingMessages: UiTranscriptMessage[],
): ThreadHistoryPaginationState {
  return mergeRemotePaginationStateWithEarliestIndex(
    current,
    incoming,
    earliestRemoteHistoryIndex(existingMessages),
  );
}

export function mergeRemotePaginationStateWithEarliestIndex(
  current: ThreadHistoryPaginationState | null,
  incoming: ThreadHistoryPaginationState,
  earliestLoadedIndex: number | null,
): ThreadHistoryPaginationState {
  if (!current) {
    return incoming;
  }
  if (!current.hasMoreBefore) {
    if (earliestLoadedIndex === 0 || !incoming.hasMoreBefore) {
      return { ...current, loadingBefore: false };
    }
    return incoming;
  }
  if (
    current.nextBeforeIndex !== null &&
    incoming.nextBeforeIndex !== null &&
    current.nextBeforeIndex <= incoming.nextBeforeIndex
  ) {
    return { ...current, loadingBefore: false };
  }
  return incoming;
}

/**
 * Fold one committed stream record forward onto the cached transcript
 * snapshot. Verbatim from applyCommittedThreadMessage's merge step.
 */
export function committedMessageForwardPage(
  base: ThreadTranscript | null,
  event: Extract<DesktopChatStreamEvent, { type: "committed_message" }>,
): ThreadTranscript {
  const threadId = event.threadId;
  const resolvedBase = base || {
    threadId,
    remoteFound: true,
    messages: [],
    pendingInputs: [],
    pageInfo: null,
  };
  return mergeForwardTranscriptPage(resolvedBase, {
    threadId,
    remoteFound: true,
    messages: [event.message],
    pendingInputs: resolvedBase.pendingInputs,
    thread: resolvedBase.thread ?? null,
    threadInfo: resolvedBase.threadInfo ?? null,
    pageInfo: {
      ...(resolvedBase.pageInfo ?? {
        totalMessages: event.seq,
        returnedMessages: 0,
        startIndex: 0,
        endIndex: event.seq,
        hasMoreBefore: false,
        nextBeforeIndex: null,
        limit: THREAD_HISTORY_PAGE_SIZE,
        userQueryLimit: THREAD_HISTORY_USER_QUERY_LIMIT,
      }),
      committedMessages: Math.max(
        event.seq,
        resolvedBase.pageInfo?.committedMessages ?? 0,
      ),
      hasMoreAfter: false,
      nextAfterIndex: null,
    },
  });
}

/**
 * Constant-history-work specialization of committedMessageForwardPage for a
 * provably contiguous raw tail append. Returning null keeps every uncertain
 * shape on the reference full fold.
 */
export function appendCommittedMessageForwardPage(
  base: ThreadTranscript | null,
  event: Extract<DesktopChatStreamEvent, { type: "committed_message" }>,
): ThreadTranscript | null {
  if (
    !base ||
    base.threadId !== event.threadId ||
    event.message.seq !== event.seq ||
    base.pageInfo?.reset === true ||
    base.pageInfo?.hasMoreAfter === true
  ) {
    return null;
  }
  const incomingIndex = transcriptMessageIndex(event.message);
  const lastMessage = base.messages[base.messages.length - 1];
  const lastIndex = lastMessage ? transcriptMessageIndex(lastMessage) : null;
  if (
    incomingIndex === null ||
    incomingIndex !== event.seq - 1 ||
    lastIndex === null ||
    incomingIndex !== lastIndex + 1
  ) {
    return null;
  }

  // Reuse the canonical forward merge for every non-message field and its
  // page-info math. Its message normalization sees only the new singleton;
  // the already-proven ordered base is appended directly.
  const metadataOnlyBase = { ...base, messages: [] };
  const merged = committedMessageForwardPage(metadataOnlyBase, event);
  return {
    ...merged,
    messages: [...base.messages, event.message],
  };
}

export interface MergeRemoteTranscriptOptions {
  activeRunLiveRows?: boolean;
  preserveRemoteBeforeIndex?: number | null;
  /**
   * Whether the fetched transcript reports an active run. Streamed local
   * tool bubbles outrank the canonical page only while the run is active.
   */
  threadRunActive?: boolean;
  intentForId: (intentId: string) => MessageIntent | null;
}

/**
 * Apply the legacy local-overlay preservation rules to a materialized remote
 * candidate set. `existing` may be the whole UI array (full reconcile) or its
 * local suffix (incremental append); priorExistingIds represents entries that
 * preceded that suffix for the legacy first-occurrence rule.
 */
export function preserveLocalTranscriptEntries(
  visibleTranscript: TranscriptMessage[],
  materializedRemote: UiTranscriptMessage[],
  existing: readonly UiTranscriptMessage[],
  options: MergeRemoteTranscriptOptions,
  priorExistingIds: ReadonlySet<string> = new Set(),
): UiTranscriptMessage[] {
  const materializedRemoteIds = new Set(
    materializedRemote.map((entry) => entry.id),
  );
  const seenExistingIds = new Set(priorExistingIds);
  const preservedLocalEntries: UiTranscriptMessage[] = [];

  for (const entry of existing) {
    const duplicate = seenExistingIds.has(entry.id);
    seenExistingIds.add(entry.id);
    if (entry.localState === "remote_final" || duplicate) {
      continue;
    }
    if (!entry.intentId) {
      if (entry.localState === "error" || entry.localState === "interrupted") {
        preservedLocalEntries.push(entry);
      }
      continue;
    }

    const intent = options.intentForId(entry.intentId);
    if (!intent) {
      if (entry.localState === "error" || entry.localState === "interrupted") {
        preservedLocalEntries.push(entry);
      }
      continue;
    }

    if (entry.role === "user") {
      if (
        !materializedRemoteIds.has(entry.id) &&
        !materializedRemoteIds.has(userMessageIdForOrigin(intent.intentId))
      ) {
        preservedLocalEntries.push(entry);
      }
      continue;
    }
    const match = resolveIntentHistoryMatch(intent, visibleTranscript);
    if (entry.role === "assistant") {
      if (!match.assistantVisible) {
        preservedLocalEntries.push(entry);
      }
      continue;
    }
    if (isToolRole(entry.role)) {
      if (
        options.threadRunActive !== false &&
        !materializedRemote.some((candidate) =>
          toolMessagesEquivalent(candidate, entry),
        )
      ) {
        preservedLocalEntries.push(entry);
      }
    }
  }
  return preservedLocalEntries;
}

export function mergeRemoteTranscriptWithLocal(
  transcript: TranscriptMessage[],
  existing: UiTranscriptMessage[],
  options: MergeRemoteTranscriptOptions,
): UiTranscriptMessage[] {
  const visibleTranscript = visibleTranscriptMessages(transcript);
  if (visibleTranscript.length === 0) {
    return existing.length > 0 ? existing : [];
  }

  const materializedRemote = materializeRemoteTranscript(
    visibleTranscript,
    existing,
    {
      ignoreTimestampForStableMessages: options?.activeRunLiveRows,
    },
  );
  const materializedRemoteIds = new Set(
    materializedRemote.map((entry) => entry.id),
  );
  const preservedRemoteBeforeEntries: UiTranscriptMessage[] = [];
  for (const entry of existing) {
    if (entry.localState === "remote_final") {
      const historyIndex = transcriptEntryHistoryIndex(entry);
      if (
        typeof options?.preserveRemoteBeforeIndex === "number" &&
        historyIndex !== null &&
        historyIndex < options.preserveRemoteBeforeIndex &&
        !materializedRemoteIds.has(entry.id)
      ) {
        preservedRemoteBeforeEntries.push(entry);
      }
    }
  }
  const preservedLocalEntries = preserveLocalTranscriptEntries(
    visibleTranscript,
    materializedRemote,
    existing,
    options,
  );

  return [
    ...preservedRemoteBeforeEntries,
    ...materializedRemote,
    ...preservedLocalEntries,
  ];
}
