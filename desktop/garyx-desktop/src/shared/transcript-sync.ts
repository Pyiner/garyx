import type {
  ThreadTranscript,
  ThreadTranscriptPageInfo,
  TranscriptMessage,
} from "./contracts";

export type TranscriptRunActivity =
  | "idle"
  | "thinking"
  | "using_tool"
  | "reconciling";

export interface TranscriptRewriteRange {
  noticeSeq?: number | null;
  startSeq: number;
  endSeq: number;
}

export interface TranscriptRunState {
  busy: boolean;
  activeRunId?: string | null;
  activity: TranscriptRunActivity;
  terminalStatus?: string | null;
  lastUserAckSeq?: number | null;
  lastUserAckPendingInputId?: string | null;
  title?: string | null;
  rewriteRanges: TranscriptRewriteRange[];
  lastTranscriptResetSeq?: number | null;
}

export type TranscriptFetchPageAction =
  | { type: "reset" }
  | { type: "shrink_refetch" }
  | {
      type: "merge_forward";
      committedOnly: boolean;
      continuePaging: boolean;
    };

export type StreamSeqDecision =
  | { type: "gap_reconnect"; resumeAfterSeq: number }
  | { type: "stale" }
  | { type: "apply" };

export type TranscriptRewriteAction = "none" | "refetch_authoritative";

const TERMINAL_CONTROL_KINDS = new Set([
  "run_complete",
  "run_interrupted",
  "interrupt_confirmed",
]);

export function shouldForwardGlobalStreamEvent(input: {
  selectedThreadId?: string | null;
  eventThreadId?: string | null;
}): boolean {
  const selectedThreadId = normalizedString(input.selectedThreadId);
  const eventThreadId = normalizedString(input.eventThreadId);
  return !selectedThreadId || !eventThreadId || selectedThreadId !== eventThreadId;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function normalizedString(value: unknown): string | null {
  return typeof value === "string" && value.trim()
    ? value.trim()
    : null;
}

function normalizedKind(value: unknown): string | null {
  const text = normalizedString(value)?.toLowerCase() || "";
  return text || null;
}

function containsToolHint(value: unknown): boolean {
  function inspect(candidate: unknown, depth: number): boolean {
    if (depth > 64) {
      return false;
    }
    if (typeof candidate === "string") {
      if (depth === 0) {
        return false;
      }
      const lower = candidate.toLowerCase();
      return (
        lower.includes("tool_use") ||
        lower.includes("tool_result") ||
        lower.includes("tool_call") ||
        lower.includes("mcp__")
      );
    }
    if (Array.isArray(candidate)) {
      return candidate.some((item) => inspect(item, depth + 1));
    }
    if (isRecord(candidate)) {
      return Object.entries(candidate).some(([key, item]) => {
        const lower = key.toLowerCase();
        return (
          lower === "tool_use_id" ||
          lower === "tool_call_id" ||
          lower === "tool_calls" ||
          lower.includes("mcp__") ||
          lower.includes("tool_") ||
          inspect(item, depth + 1)
        );
      });
    }
    return false;
  }
  return inspect(value, 0);
}

function isToolRelatedTranscriptMessage(
  message: Pick<
    TranscriptMessage,
    "role" | "content" | "toolName" | "toolRelated" | "metadata"
  >,
): boolean {
  if (
    message.role === "tool_use" ||
    message.role === "tool_result"
  ) {
    return true;
  }
  if (message.toolRelated) {
    return true;
  }
  if (normalizedString(message.toolName)) {
    return true;
  }
  return containsToolHint(message.content) || containsToolHint(message.metadata);
}

function controlObject(message: Pick<TranscriptMessage, "content">): Record<
  string,
  unknown
> | null {
  const content = message.content;
  if (!isRecord(content)) {
    return null;
  }
  const nested = content.control;
  if (isRecord(nested)) {
    return nested;
  }
  return null;
}

export function transcriptControlKind(
  message: Pick<TranscriptMessage, "kind" | "role" | "content">,
): string | null {
  const content = isRecord(message.content) ? message.content : null;
  const directKind = normalizedKind(content?.kind);
  const internalKind = normalizedKind(content?.internal_kind);
  const isControlEnvelope =
    message.kind === "control" ||
    directKind === "control" ||
    internalKind === "control" ||
    (message.role === "system" && isRecord(content?.control));
  if (!isControlEnvelope) {
    return null;
  }
  return normalizedKind(controlObject(message)?.kind) || directKind;
}

export function isControlTranscriptMessage(
  message: Pick<TranscriptMessage, "kind" | "role" | "content">,
): boolean {
  return transcriptControlKind(message) !== null;
}

export function transcriptMessageIndex(
  message: Pick<TranscriptMessage, "id">,
): number | null {
  const suffix = String(message.id || "").split(":").pop() || "";
  if (!/^\d+$/.test(suffix)) {
    return null;
  }
  const value = Number(suffix);
  return Number.isSafeInteger(value) ? value : null;
}

export function transcriptAfterCursor(messages: TranscriptMessage[]): number | null {
  let cursor: number | null = null;
  for (const message of messages) {
    const index = transcriptMessageIndex(message);
    if (index === null) {
      continue;
    }
    cursor = cursor === null ? index : Math.max(cursor, index);
  }
  return cursor;
}

export function decideTranscriptFetchPageAction(input: {
  cursor: number;
  reset?: boolean | null;
  hasMoreAfter?: boolean | null;
  totalMessagesInThread?: number | null;
}): TranscriptFetchPageAction {
  if (input.reset) {
    return { type: "reset" };
  }
  if (
    typeof input.totalMessagesInThread === "number" &&
    input.cursor >= input.totalMessagesInThread
  ) {
    return { type: "shrink_refetch" };
  }
  return {
    type: "merge_forward",
    committedOnly: Boolean(input.hasMoreAfter),
    continuePaging: Boolean(input.hasMoreAfter),
  };
}

export function decideStreamSeq(input: {
  incomingSeq: number;
  connectionLastSeq: number;
}): StreamSeqDecision {
  if (
    input.connectionLastSeq > 0 &&
    input.incomingSeq > input.connectionLastSeq + 1
  ) {
    return {
      type: "gap_reconnect",
      resumeAfterSeq: input.connectionLastSeq,
    };
  }
  if (input.incomingSeq < input.connectionLastSeq) {
    return { type: "stale" };
  }
  return { type: "apply" };
}

export function streamResumeCursor(input: {
  afterCursor?: number | null;
  fallbackMaxIndex?: number | null;
}): number {
  if (typeof input.afterCursor === "number") {
    return input.afterCursor + 1;
  }
  if (typeof input.fallbackMaxIndex === "number") {
    return input.fallbackMaxIndex + 1;
  }
  return 0;
}

export function normalizeCommittedTranscriptMessages(
  existing: TranscriptMessage[],
  fetched: TranscriptMessage[],
): TranscriptMessage[] {
  const byIndex = new Map<number, TranscriptMessage>();
  for (const message of [...existing, ...fetched]) {
    const index = transcriptMessageIndex(message);
    if (index === null) {
      continue;
    }
    byIndex.set(index, message);
  }
  return [...byIndex.entries()]
    .sort(([left], [right]) => left - right)
    .map(([, message]) => message);
}

function mergeForwardPageInfo(
  base: ThreadTranscriptPageInfo | null | undefined,
  page: ThreadTranscriptPageInfo | null | undefined,
): ThreadTranscriptPageInfo | null {
  if (!base && !page) {
    return null;
  }
  return {
    totalMessages: page?.totalMessages ?? base?.totalMessages ?? 0,
    committedMessages:
      page?.committedMessages ?? base?.committedMessages ?? null,
    overlayMessages: page?.overlayMessages ?? base?.overlayMessages ?? null,
    returnedMessages: page?.returnedMessages ?? base?.returnedMessages ?? 0,
    returnedUserQueries:
      page?.returnedUserQueries ?? base?.returnedUserQueries ?? null,
    startIndex: base?.startIndex ?? page?.startIndex ?? 0,
    endIndex: page?.endIndex ?? base?.endIndex ?? 0,
    hasMoreBefore: base?.hasMoreBefore ?? page?.hasMoreBefore ?? false,
    nextBeforeIndex: base?.nextBeforeIndex ?? page?.nextBeforeIndex ?? null,
    hasMoreAfter: page?.hasMoreAfter ?? false,
    nextAfterIndex: page?.nextAfterIndex ?? null,
    reset: page?.reset ?? false,
    limit: page?.limit ?? base?.limit ?? 0,
    userQueryLimit: page?.userQueryLimit ?? base?.userQueryLimit ?? null,
  };
}

export function mergeForwardTranscriptPage(
  base: ThreadTranscript | null,
  page: ThreadTranscript,
): ThreadTranscript {
  if (!base) {
    return page;
  }
  return {
    ...base,
    remoteFound: page.remoteFound || base.remoteFound,
    messages: normalizeCommittedTranscriptMessages(base.messages, page.messages),
    pendingInputs: page.pendingInputs,
    thread: page.thread ?? base.thread ?? null,
    threadInfo: page.threadInfo ?? base.threadInfo ?? null,
    pageInfo: mergeForwardPageInfo(base.pageInfo, page.pageInfo),
    team: page.team ?? base.team ?? null,
  };
}

export function committedTranscriptMessages(
  transcript: ThreadTranscript,
): TranscriptMessage[] {
  const committedMessages = transcript.pageInfo?.committedMessages;
  if (typeof committedMessages === "number" && Number.isFinite(committedMessages)) {
    return transcript.messages.filter((message) => {
      const index = transcriptMessageIndex(message);
      return index !== null && index < committedMessages;
    });
  }
  if (transcript.pageInfo?.hasMoreAfter) {
    return transcript.messages;
  }
  if (transcript.threadInfo?.activeRun) {
    return [];
  }
  return transcript.messages;
}

export function transcriptCommittedAfterCursor(
  transcript: ThreadTranscript | null,
): number | null {
  if (!transcript) {
    return null;
  }
  return transcriptAfterCursor(committedTranscriptMessages(transcript));
}

export function transcriptForCommittedCache(
  transcript: ThreadTranscript,
): ThreadTranscript {
  const messages = committedTranscriptMessages(transcript);
  const afterCursor = transcriptAfterCursor(messages);
  const firstCursor =
    messages
      .map((message) => transcriptMessageIndex(message))
      .filter((value): value is number => value !== null)
      .sort((left, right) => left - right)[0] ?? null;
  return {
    ...transcript,
    messages,
    pageInfo: transcript.pageInfo
      ? {
          ...transcript.pageInfo,
          returnedMessages: messages.length,
          startIndex: firstCursor ?? transcript.pageInfo.startIndex,
          endIndex:
            afterCursor === null
              ? firstCursor ?? transcript.pageInfo.startIndex
              : afterCursor + 1,
          hasMoreAfter: false,
          nextAfterIndex: null,
          overlayMessages: 0,
        }
      : transcript.pageInfo,
  };
}

export function transcriptWithResolvedActiveRun(
  transcript: ThreadTranscript,
): ThreadTranscript {
  const threadInfo = transcript.threadInfo;
  const activeRunId = normalizedString(threadInfo?.activeRun?.runId);
  if (!threadInfo || !activeRunId) {
    return transcript;
  }

  let activeRunTerminated = false;
  for (const message of transcript.messages) {
    const control = controlObject(message);
    const controlRunId =
      normalizedString(control?.run_id) || normalizedString(control?.runId);
    if (controlRunId !== activeRunId) {
      continue;
    }
    const kind = normalizedKind(control?.kind);
    if (!kind) {
      continue;
    }
    if (kind === "run_start") {
      activeRunTerminated = false;
      continue;
    }
    if (TERMINAL_CONTROL_KINDS.has(kind)) {
      activeRunTerminated = true;
    }
  }

  if (!activeRunTerminated) {
    return transcript;
  }

  return {
    ...transcript,
    threadInfo: {
      ...threadInfo,
      activeRun: null,
    },
  };
}

export function shouldRestartSelectedThreadStreamAfterRefetch(input: {
  threadId?: string | null;
  selectedThreadId?: string | null;
  startSelectionGeneration: number;
  currentSelectionGeneration: number;
}): boolean {
  const threadId = normalizedString(input.threadId);
  const selectedThreadId = normalizedString(input.selectedThreadId);
  return Boolean(
    threadId &&
      selectedThreadId &&
      threadId === selectedThreadId &&
      input.startSelectionGeneration === input.currentSelectionGeneration,
  );
}

export function shouldRefetchAuthoritativeAfterForwardPageLimit(input: {
  pagesFetched: number;
  maxPages: number;
  hasMoreAfter?: boolean | null;
}): boolean {
  return (
    input.pagesFetched >= input.maxPages &&
    input.maxPages > 0 &&
    Boolean(input.hasMoreAfter)
  );
}

export function isThreadStreamGapError(input: {
  runId?: string | null;
  error?: string | null;
}): boolean {
  return (
    normalizedString(input.runId) === "thread-stream-gap" ||
    (normalizedString(input.error)?.toLowerCase().includes(
      "thread stream seq gap",
    ) ??
      false)
  );
}

export function deriveTranscriptKind(
  message: Pick<
    TranscriptMessage,
    | "kind"
    | "role"
    | "content"
    | "toolName"
    | "toolRelated"
    | "metadata"
  >,
): string {
  if (isControlTranscriptMessage(message)) {
    return "control";
  }
  const toolRelated = isToolRelatedTranscriptMessage(message);
  if (
    message.role === "tool_use" ||
    message.role === "tool_result" ||
    toolRelated
  ) {
    return "tool_trace";
  }
  if (message.role === "assistant") {
    return "assistant_reply";
  }
  if (message.role === "user") {
    return "user_input";
  }
  return message.role || "unknown";
}

function numericControlValue(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) {
    return Math.max(0, Math.trunc(value));
  }
  if (typeof value === "string" && /^\d+$/.test(value.trim())) {
    return Number(value.trim());
  }
  return null;
}

function runStateInitial(): TranscriptRunState {
  return {
    busy: false,
    activeRunId: null,
    activity: "idle",
    terminalStatus: null,
    lastUserAckSeq: null,
    lastUserAckPendingInputId: null,
    title: null,
    rewriteRanges: [],
    lastTranscriptResetSeq: null,
  };
}

export function reduceTranscriptRunState(
  messages: TranscriptMessage[],
): TranscriptRunState {
  const state = runStateInitial();
  for (const message of messages) {
    const seq = transcriptMessageIndex(message);
    const kind = deriveTranscriptKind(message);
    if (kind === "control") {
      applyControlMessage(state, seq === null ? null : seq + 1, message);
      continue;
    }
    if (kind === "tool_trace") {
      if (state.busy) {
        state.activity = "using_tool";
      }
      continue;
    }
    if (
      (kind === "assistant_reply" || kind === "user_input") &&
      state.busy &&
      state.activity !== "reconciling"
    ) {
      state.activity = "thinking";
    }
  }
  return state;
}

function applyControlMessage(
  state: TranscriptRunState,
  seq: number | null,
  message: Pick<TranscriptMessage, "content">,
): void {
  const control = controlObject(message);
  const kind = normalizedKind(control?.kind);
  if (!control || !kind) {
    return;
  }
  switch (kind) {
    case "run_start":
      state.busy = true;
      state.activeRunId = normalizedString(control.run_id) || null;
      state.terminalStatus = null;
      state.activity = "thinking";
      break;
    case "user_ack":
      state.lastUserAckSeq = seq;
      state.lastUserAckPendingInputId =
        normalizedString(control.pending_input_id) ||
        normalizedString(control.pendingInputId) ||
        null;
      break;
    case "assistant_boundary":
      if (state.busy && state.activity !== "reconciling") {
        state.activity = "thinking";
      }
      break;
    case "done":
      if (state.busy) {
        state.activity = "reconciling";
      }
      break;
    case "run_complete":
      state.busy = false;
      state.activeRunId = null;
      state.activity = "idle";
      state.terminalStatus = normalizedString(control.status) || "completed";
      break;
    case "run_interrupted":
    case "interrupt_confirmed":
      state.busy = false;
      state.activeRunId = null;
      state.activity = "idle";
      state.terminalStatus = "interrupted";
      break;
    case "thread_title_updated":
      state.title = normalizedString(control.title);
      break;
    case "transcript_reset":
      state.lastTranscriptResetSeq = seq;
      break;
    case "range_rewrite": {
      const startSeq = numericControlValue(control.start_seq) ?? seq ?? 0;
      const endSeq = numericControlValue(control.end_seq) ?? startSeq;
      state.rewriteRanges.push({
        noticeSeq: seq,
        startSeq,
        endSeq,
      });
      break;
    }
    default:
      break;
  }
}

export function transcriptRewriteAction(
  message: Pick<TranscriptMessage, "kind" | "role" | "content">,
): TranscriptRewriteAction {
  const kind = transcriptControlKind(message);
  return kind === "range_rewrite" || kind === "transcript_reset"
    ? "refetch_authoritative"
    : "none";
}
