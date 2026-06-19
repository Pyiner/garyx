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
  pendingToolCallIds: Record<string, number>;
  pendingAnonymousToolCallCount: number;
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
    | "role"
    | "content"
    | "input"
    | "result"
    | "toolName"
    | "toolRelated"
    | "metadata"
  >,
): boolean {
  if (
    message.role === "tool" ||
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
  return containsToolHint(message.content) ||
    containsToolHint(message.metadata) ||
    containsToolHint(message.input) ||
    containsToolHint(message.result);
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
    | "input"
    | "result"
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
    message.role === "tool" ||
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
  if (message.role === "system") {
    return "system";
  }
  return "internal";
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

export function initialTranscriptRunState(): TranscriptRunState {
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
    pendingToolCallIds: {},
    pendingAnonymousToolCallCount: 0,
  };
}

export function reduceTranscriptRunState(
  messages: TranscriptMessage[],
): TranscriptRunState {
  let state = initialTranscriptRunState();
  for (const message of messages) {
    state = applyTranscriptRunStateRecord(state, message);
  }
  return state;
}

export function applyTranscriptRunStateRecord(
  state: TranscriptRunState,
  message: TranscriptMessage,
  options?: { seq?: number | null },
): TranscriptRunState {
  const next: TranscriptRunState = {
    ...state,
    rewriteRanges: [...state.rewriteRanges],
    pendingToolCallIds: { ...state.pendingToolCallIds },
    pendingAnonymousToolCallCount: state.pendingAnonymousToolCallCount,
  };
  const index = transcriptMessageIndex(message);
  const seq = options && "seq" in options
    ? options.seq ?? null
    : index === null
      ? null
      : index + 1;
  const kind = deriveTranscriptKind(message);
  if (kind === "control") {
    applyControlMessage(next, seq, message);
    return next;
  }
  if (kind === "tool_trace") {
    if (next.busy && next.activity !== "reconciling") {
      applyToolTraceMessage(next, message);
    }
    return next;
  }
  if (
    (kind === "assistant_reply" || kind === "user_input") &&
    next.busy &&
    next.activity !== "reconciling"
  ) {
    next.activity = activityForPendingTools(next);
  }
  return next;
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
      clearPendingTools(state);
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
        state.activity = activityForPendingTools(state);
      }
      break;
    case "done":
      if (state.busy) {
        clearPendingTools(state);
        state.activity = "reconciling";
      }
      break;
    case "run_complete":
      state.busy = false;
      state.activeRunId = null;
      clearPendingTools(state);
      state.activity = "idle";
      state.terminalStatus = normalizedString(control.status) || "completed";
      break;
    case "run_interrupted":
    case "interrupt_confirmed":
      state.busy = false;
      state.activeRunId = null;
      clearPendingTools(state);
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

function applyToolTraceMessage(
  state: TranscriptRunState,
  message: Pick<
    TranscriptMessage,
    "role" | "content" | "input" | "result" | "toolUseId" | "toolUseResult"
  >,
): void {
  if (isToolResultTrace(message)) {
    markToolResult(state, toolCallId(message));
  } else {
    markToolUse(state, toolCallId(message));
  }
  state.activity = activityForPendingTools(state);
}

function isToolResultTrace(
  message: Pick<
    TranscriptMessage,
    "role" | "content" | "result" | "toolUseResult"
  >,
): boolean {
  if (message.role === "tool" || message.role === "tool_result") {
    return true;
  }
  if (message.toolUseResult) {
    return true;
  }
  if (message.result !== undefined && message.result !== null) {
    return true;
  }
  return toolTraceTypeIsResult(message.content);
}

function toolTraceTypeIsResult(value: unknown): boolean {
  if (!isRecord(value)) {
    return false;
  }
  return normalizedKind(value.type) === "tool_result" ||
    normalizedKind(value.kind) === "tool_result";
}

function markToolUse(
  state: TranscriptRunState,
  toolCallId: string | null,
): void {
  if (toolCallId) {
    state.pendingToolCallIds[toolCallId] =
      (state.pendingToolCallIds[toolCallId] ?? 0) + 1;
  } else {
    state.pendingAnonymousToolCallCount += 1;
  }
}

function markToolResult(
  state: TranscriptRunState,
  toolCallId: string | null,
): void {
  if (toolCallId) {
    decrementPendingToolId(state, toolCallId);
    return;
  }
  if (state.pendingAnonymousToolCallCount > 0) {
    state.pendingAnonymousToolCallCount -= 1;
  } else if (Object.keys(state.pendingToolCallIds).length === 1) {
    state.pendingToolCallIds = {};
  }
}

function decrementPendingToolId(
  state: TranscriptRunState,
  toolCallId: string,
): boolean {
  const count = state.pendingToolCallIds[toolCallId];
  if (!count) {
    return false;
  }
  if (count <= 1) {
    delete state.pendingToolCallIds[toolCallId];
  } else {
    state.pendingToolCallIds[toolCallId] = count - 1;
  }
  return true;
}

function toolCallId(
  message: Pick<TranscriptMessage, "toolUseId" | "content" | "input" | "result">,
): string | null {
  return normalizedString(message.toolUseId) ||
    nestedToolCallId(message.content) ||
    nestedToolCallId(message.input) ||
    nestedToolCallId(message.result);
}

function nestedToolCallId(value: unknown): string | null {
  if (!isRecord(value)) {
    return null;
  }
  return normalizedString(value.tool_use_id) ||
    normalizedString(value.toolUseId) ||
    normalizedString(value.tool_call_id) ||
    normalizedString(value.toolCallId) ||
    normalizedString(value.id);
}

function activityForPendingTools(
  state: Pick<
    TranscriptRunState,
    "pendingToolCallIds" | "pendingAnonymousToolCallCount"
  >,
): TranscriptRunActivity {
  return state.pendingAnonymousToolCallCount > 0 ||
      Object.keys(state.pendingToolCallIds).length > 0
    ? "using_tool"
    : "thinking";
}

function clearPendingTools(state: TranscriptRunState): void {
  state.pendingToolCallIds = {};
  state.pendingAnonymousToolCallCount = 0;
}

export function transcriptRewriteAction(
  message: Pick<TranscriptMessage, "kind" | "role" | "content">,
): TranscriptRewriteAction {
  const kind = transcriptControlKind(message);
  return kind === "range_rewrite" || kind === "transcript_reset"
    ? "refetch_authoritative"
    : "none";
}

// Tool-message identity helpers. These are transport/reconciliation utilities,
// not rendering logic: optimistic and gateway-recovery reconciliation use them
// to match tool messages across local and remote copies.

function isToolUtilRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

export function isToolRole(
  role: TranscriptMessage["role"],
): role is "tool_use" | "tool_result" {
  return role === "tool_use" || role === "tool_result";
}

export function extractToolUseId(message: TranscriptMessage): string | null {
  if (message.toolUseId) return message.toolUseId;
  const content = message.content;
  if (isToolUtilRecord(content)) {
    const id =
      (typeof content.tool_use_id === "string" && content.tool_use_id) ||
      (typeof content.toolUseId === "string" && content.toolUseId);
    if (id) return id;
    if (isToolUtilRecord(content.content)) {
      const innerId =
        (typeof content.content.tool_use_id === "string" &&
          content.content.tool_use_id) ||
        (typeof content.content.toolUseId === "string" &&
          content.content.toolUseId);
      if (innerId) return innerId;
    }
  }
  if (typeof message.text === "string") {
    try {
      const parsed = JSON.parse(message.text);
      if (isToolUtilRecord(parsed)) {
        const id =
          (typeof parsed.tool_use_id === "string" && parsed.tool_use_id) ||
          (typeof parsed.toolUseId === "string" && parsed.toolUseId);
        if (id) return id;
      }
    } catch {
      // plain text, ignore
    }
  }
  return null;
}

function stableSerializeToolValue(value: unknown): string {
  if (value === null || value === undefined) {
    return "";
  }
  if (typeof value === "string") {
    return value;
  }
  if (typeof value !== "object") {
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map((entry) => stableSerializeToolValue(entry)).join(",")}]`;
  }
  const entries = Object.entries(value)
    .filter(([, entryValue]) => entryValue !== undefined)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(
      ([key, entryValue]) =>
        `${JSON.stringify(key)}:${stableSerializeToolValue(entryValue)}`,
    );
  return `{${entries.join(",")}}`;
}

function toolMessageFingerprint(message: TranscriptMessage): string {
  return [
    message.role,
    extractToolUseId(message) || "",
    message.toolName || "",
    message.isError ? "1" : "0",
    message.text || "",
    stableSerializeToolValue(message.content),
    stableSerializeToolValue(message.metadata),
  ].join("::");
}

export function toolMessagesEquivalent(
  left: TranscriptMessage,
  right: TranscriptMessage,
): boolean {
  if (
    left.role !== right.role ||
    !isToolRole(left.role) ||
    !isToolRole(right.role)
  ) {
    return false;
  }

  const leftToolUseId = extractToolUseId(left);
  const rightToolUseId = extractToolUseId(right);
  if (leftToolUseId && rightToolUseId) {
    return leftToolUseId === rightToolUseId;
  }

  return toolMessageFingerprint(left) === toolMessageFingerprint(right);
}
