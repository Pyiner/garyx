import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import {
  committedTranscriptMessages,
  decideStreamSeq,
  decideTranscriptFetchPageAction,
  deriveTranscriptKind,
  isThreadStreamGapError,
  applyTranscriptRunStateRecord,
  mergeForwardTranscriptPage,
  reduceTranscriptRunState,
  shouldRefetchAuthoritativeAfterForwardPageLimit,
  shouldRestartSelectedThreadStreamAfterRefetch,
  streamResumeCursor,
  transcriptAfterCursor,
  transcriptCommittedAfterCursor,
  transcriptForCommittedCache,
  transcriptRewriteAction,
  transcriptWithResolvedActiveRun,
} from "./transcript-sync.ts";

const fixtureRoot = join(
  process.cwd(),
  "../../test-fixtures/stream-sync",
);

function readJsonl(name) {
  return readFileSync(join(fixtureRoot, name), "utf8")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

function normalizeRole(role) {
  return role === "assistant" ||
    role === "user" ||
    role === "tool_use" ||
    role === "tool_result"
    ? role
    : "system";
}

function contentMessage(index, threadId, raw) {
  const message = raw.message ?? raw;
  const role = normalizeRole(message.role);
  const metadata = message.metadata && typeof message.metadata === "object"
    ? message.metadata
    : null;
  const content = Object.prototype.hasOwnProperty.call(message, "content")
    ? message.content
    : null;
  const kind =
    role === "tool_use" || role === "tool_result"
      ? "tool_trace"
      : role === "assistant"
        ? "assistant_reply"
        : role === "user"
          ? "user_input"
          : message.kind;
  return {
    id: `${threadId}:${index}`,
    role,
    text:
      typeof message.text === "string"
        ? message.text
        : typeof content === "string"
          ? content
          : "",
    content,
    timestamp: message.timestamp ?? raw.timestamp ?? null,
    toolUseId: message.tool_use_id ?? message.toolUseId ?? null,
    toolName: message.tool_name ?? message.toolName ?? metadata?.item_type ?? null,
    isError: message.is_error ?? message.isError,
    metadata,
    kind,
    internal: Boolean(message.internal),
    internalKind: message.internal_kind ?? message.internalKind ?? null,
  };
}

function controlMessage(index, event, overrides = {}) {
  const threadId =
    event.thread_id ?? event.threadId ?? "thread::fixture-stream-sync";
  const runId = event.run_id ?? event.runId ?? "run::fixture";
  const type = overrides.kind ?? event.type;
  const control = {
    kind: type,
    thread_id: threadId,
    run_id: runId,
    at: "2026-06-18T12:00:00Z",
  };
  if (event.pending_input_id ?? event.pendingInputId) {
    control.pending_input_id = event.pending_input_id ?? event.pendingInputId;
  }
  if (event.duration_ms ?? event.durationMs) {
    control.duration_ms = event.duration_ms ?? event.durationMs;
  }
  if (event.title) {
    control.title = event.title;
  }
  Object.assign(control, overrides.control ?? {});
  return {
    id: `${threadId}:${index}`,
    role: "system",
    text: "",
    content: {
      role: "system",
      kind: "control",
      internal: true,
      internal_kind: "control",
      control,
    },
    timestamp: "2026-06-18T12:00:00Z",
    kind: "control",
    internal: true,
    internalKind: "control",
  };
}

const COMMITTED_THREAD_ID = "thread::fixture-committed-run-state";
const COMMITTED_RUN_ID = "run::fixture-committed-run-state";
const COMMITTED_TIMESTAMP = "2026-06-18T12:00:00Z";

function committedMessagePayload(seq, message, overrides = {}) {
  return {
    type: "committed_message",
    thread_id: overrides.threadId ?? COMMITTED_THREAD_ID,
    run_id: overrides.runId ?? COMMITTED_RUN_ID,
    seq,
    timestamp: overrides.timestamp ?? COMMITTED_TIMESTAMP,
    message: {
      timestamp: overrides.timestamp ?? COMMITTED_TIMESTAMP,
      ...message,
    },
  };
}

function committedControlPayload(seq, kind, control = {}) {
  return committedMessagePayload(seq, {
    role: "system",
    kind: "control",
    internal: true,
    internal_kind: "control",
    control: {
      kind,
      thread_id: COMMITTED_THREAD_ID,
      run_id: COMMITTED_RUN_ID,
      at: COMMITTED_TIMESTAMP,
      ...control,
    },
  });
}

function committedPayloadToTranscriptMessage(payload) {
  const rawMessage = payload.message;
  const role = normalizeRole(rawMessage.role);
  const metadata = rawMessage.metadata && typeof rawMessage.metadata === "object"
    ? rawMessage.metadata
    : null;
  const kind =
    rawMessage.kind ??
    (rawMessage.internal_kind === "control"
      ? "control"
      : role === "tool_use" || role === "tool_result"
        ? "tool_trace"
        : role === "assistant"
          ? "assistant_reply"
          : role === "user"
            ? "user_input"
            : undefined);
  const isControlRecord =
    kind === "control" || rawMessage.internal_kind === "control";
  return {
    id: `${payload.thread_id}:${payload.seq - 1}`,
    role: isControlRecord ? "system" : role,
    text: isControlRecord
      ? ""
      : typeof rawMessage.text === "string"
        ? rawMessage.text
        : typeof rawMessage.content === "string"
          ? rawMessage.content
          : "",
    content: isControlRecord ? rawMessage : rawMessage.content,
    input: rawMessage.input,
    result: rawMessage.result,
    timestamp: rawMessage.timestamp ?? payload.timestamp ?? null,
    toolUseId: rawMessage.tool_use_id ?? rawMessage.toolUseId ?? null,
    toolName:
      rawMessage.tool_name ??
      rawMessage.toolName ??
      metadata?.item_type ??
      metadata?.itemType ??
      null,
    isError: rawMessage.is_error ?? rawMessage.isError,
    toolUseResult: rawMessage.tool_use_result ?? rawMessage.toolUseResult ?? null,
    metadata,
    kind: isControlRecord ? "control" : kind,
    internal: isControlRecord || Boolean(rawMessage.internal),
    internalKind:
      rawMessage.internal_kind ??
      rawMessage.internalKind ??
      (isControlRecord ? "control" : null),
  };
}

function committedRunStateMessages() {
  return [
    committedControlPayload(1, "run_start"),
    committedControlPayload(2, "user_ack", {
      pending_input_id: "pending-fixture-followup",
    }),
    committedMessagePayload(3, {
      role: "tool_use",
      kind: "tool_trace",
      content: {
        type: "tool_use",
        tool_use_id: "call-read-design",
        input: { path: "docs/design.md" },
      },
      tool_use_id: "call-read-design",
      tool_name: "Read",
    }),
    committedMessagePayload(4, {
      role: "tool_result",
      kind: "tool_trace",
      content: {
        type: "tool_result",
        tool_use_id: "call-read-design",
        result: "Read 334 lines",
      },
      tool_use_id: "call-read-design",
      tool_name: "Read",
    }),
    committedControlPayload(5, "done"),
    committedControlPayload(6, "thread_title_updated", {
      title: "Committed run title",
    }),
    committedControlPayload(7, "run_complete", {
      status: "completed",
      duration_ms: 1234,
    }),
  ].map(committedPayloadToTranscriptMessage);
}

test("stream seq planner applies first replay row, skips stale rows, and reconnects gaps", () => {
  assert.deepEqual(
    decideStreamSeq({ incomingSeq: 100, connectionLastSeq: 0 }),
    { type: "apply" },
  );
  assert.deepEqual(
    decideStreamSeq({ incomingSeq: 6, connectionLastSeq: 5 }),
    { type: "apply" },
  );
  assert.deepEqual(
    decideStreamSeq({ incomingSeq: 8, connectionLastSeq: 5 }),
    { type: "gap_reconnect", resumeAfterSeq: 5 },
  );
  assert.deepEqual(
    decideStreamSeq({ incomingSeq: 3, connectionLastSeq: 5 }),
    { type: "stale" },
  );
  assert.equal(
    streamResumeCursor({ afterCursor: 10, fallbackMaxIndex: null }),
    11,
  );
  assert.equal(
    streamResumeCursor({ afterCursor: null, fallbackMaxIndex: 20 }),
    21,
  );
});

test("after_index planner resets, refetches shrink, and pages forward", () => {
  assert.deepEqual(
    decideTranscriptFetchPageAction({
      cursor: 3,
      reset: true,
      hasMoreAfter: false,
      totalMessagesInThread: 10,
    }),
    { type: "reset" },
  );
  assert.deepEqual(
    decideTranscriptFetchPageAction({
      cursor: 8,
      reset: false,
      hasMoreAfter: false,
      totalMessagesInThread: 8,
    }),
    { type: "shrink_refetch" },
  );
  assert.deepEqual(
    decideTranscriptFetchPageAction({
      cursor: 3,
      reset: false,
      hasMoreAfter: true,
      totalMessagesInThread: 10,
    }),
    { type: "merge_forward", committedOnly: true, continuePaging: true },
  );
});

test("committed message records replay to idle terminal state", () => {
  const state = reduceTranscriptRunState(committedRunStateMessages());
  assert.equal(state.busy, false);
  assert.equal(state.activity, "idle");
  assert.equal(state.terminalStatus, "completed");
  assert.equal(state.activeRunId, null);
  assert.equal(state.title, "Committed run title");
  assert.equal(state.lastUserAckPendingInputId, "pending-fixture-followup");
  assert.equal(state.lastUserAckSeq, 2);
});

test("incremental run-state apply matches full reducer on committed messages", () => {
  const messages = committedRunStateMessages();
  let incremental = reduceTranscriptRunState([]);
  const activities = [];
  for (let index = 0; index < messages.length; index += 1) {
    incremental = applyTranscriptRunStateRecord(incremental, messages[index], {
      seq: index + 1,
    });
    activities.push(incremental.activity);
  }
  assert.deepEqual(activities, [
    "thinking",
    "thinking",
    "using_tool",
    "thinking",
    "reconciling",
    "reconciling",
    "idle",
  ]);
  assert.deepEqual(incremental, reduceTranscriptRunState(messages));
});

test("multi-tool lull fixture replays finished tool gaps as thinking", () => {
  const messages = readJsonl("multi-tool-lull.jsonl")
    .filter((record) => record.type === "committed_message")
    .map(committedPayloadToTranscriptMessage);

  const firstToolLull = reduceTranscriptRunState(messages.slice(0, 4));
  assert.equal(firstToolLull.busy, true);
  assert.equal(firstToolLull.activity, "thinking");

  const secondToolRunning = reduceTranscriptRunState(messages.slice(0, 5));
  assert.equal(secondToolRunning.busy, true);
  assert.equal(secondToolRunning.activity, "using_tool");

  const finalToolLull = reduceTranscriptRunState(messages.slice(0, 6));
  assert.equal(finalToolLull.busy, true);
  assert.equal(finalToolLull.activity, "thinking");
});

test("parallel tool lull fixture waits for all results before thinking", () => {
  const messages = readJsonl("parallel-tool-lull.jsonl")
    .filter((record) => record.type === "committed_message")
    .map(committedPayloadToTranscriptMessage);

  const bothToolsRunning = reduceTranscriptRunState(messages.slice(0, 4));
  assert.equal(bothToolsRunning.busy, true);
  assert.equal(bothToolsRunning.activity, "using_tool");

  const oneToolStillRunning = reduceTranscriptRunState(messages.slice(0, 5));
  assert.equal(oneToolStillRunning.busy, true);
  assert.equal(oneToolStillRunning.activity, "using_tool");

  const allToolsFinished = reduceTranscriptRunState(messages.slice(0, 6));
  assert.equal(allToolsFinished.busy, true);
  assert.equal(allToolsFinished.activity, "thinking");
});

test("transcript kind resolver matches control and tool fixture semantics", () => {
  const toolRecords = readJsonl("transcript-with-tool.jsonl");
  const messages = toolRecords.map((record, index) =>
    contentMessage(index, record.thread_id, record),
  );
  assert.equal(deriveTranscriptKind(messages[0]), "user_input");
  assert.equal(deriveTranscriptKind(messages[1]), "assistant_reply");
  assert.equal(deriveTranscriptKind(messages[2]), "tool_trace");
  assert.equal(deriveTranscriptKind(messages[3]), "tool_trace");
  assert.equal(deriveTranscriptKind(messages[4]), "assistant_reply");

  assert.equal(
    deriveTranscriptKind({
      id: "thread::kind:0",
      role: "assistant",
      text: "this text mentions tool_use and mcp__ without a structured payload",
      content: "this text mentions tool_use and mcp__ without a structured payload",
    }),
    "assistant_reply",
  );
  assert.equal(
    deriveTranscriptKind({
      id: "thread::kind:1",
      role: "assistant",
      text: "",
      content: { tool_use_id: "call-1", input: {} },
    }),
    "tool_trace",
  );
  assert.equal(
    deriveTranscriptKind({
      id: "thread::kind:2",
      role: "assistant",
      text: "",
      input: { tool_calls: [{ id: "call-2" }] },
    }),
    "tool_trace",
  );
  assert.equal(
    deriveTranscriptKind({
      id: "thread::kind:3",
      role: "assistant",
      text: "",
      result: { tool_use_id: "call-3" },
    }),
    "tool_trace",
  );
  assert.equal(
    deriveTranscriptKind({
      id: "thread::kind:4",
      role: "tool",
      text: "",
    }),
    "tool_trace",
  );
  assert.equal(
    deriveTranscriptKind({
      id: "thread::kind:5",
      role: "developer",
      text: "internal provider note",
    }),
    "internal",
  );
  assert.equal(
    deriveTranscriptKind(controlMessage(0, {
      type: "run_start",
      threadId: "thread::kind",
    })),
    "control",
  );
});

test("rewrite controls surface invalidation windows", () => {
  const rangeRewrite = controlMessage(
    1,
    { type: "range_rewrite", threadId: "thread::fixture-rewrite" },
    { control: { start_seq: 1, end_seq: 1 } },
  );
  const transcriptReset = controlMessage(2, {
    type: "transcript_reset",
    threadId: "thread::fixture-rewrite",
  });
  assert.equal(transcriptRewriteAction(rangeRewrite), "refetch_authoritative");
  assert.equal(transcriptRewriteAction(transcriptReset), "refetch_authoritative");
  const state = reduceTranscriptRunState([
    controlMessage(0, {
      type: "run_start",
      threadId: "thread::fixture-rewrite",
    }),
    rangeRewrite,
    transcriptReset,
  ]);
  assert.deepEqual(state.rewriteRanges, [
    { noticeSeq: 2, startSeq: 1, endSeq: 1 },
  ]);
  assert.equal(state.lastTranscriptResetSeq, 3);
});

test("active-run live rows do not advance committed cache cursor", () => {
  const threadId = "thread::fixture-live";
  const transcript = {
    threadId,
    remoteFound: true,
    messages: [
      contentMessage(0, threadId, {
        message: { role: "user", content: "q", text: "q" },
      }),
      contentMessage(1, threadId, {
        message: { role: "assistant", content: "stable", text: "stable" },
      }),
      contentMessage(2, threadId, {
        message: { role: "assistant", content: "streaming", text: "streaming" },
      }),
    ],
    pendingInputs: [],
    threadInfo: {
      activeRun: { runId: "run::live" },
      channelBindings: [],
    },
    pageInfo: {
      totalMessages: 3,
      committedMessages: 2,
      returnedMessages: 3,
      startIndex: 0,
      endIndex: 3,
      hasMoreBefore: false,
      nextBeforeIndex: null,
      hasMoreAfter: false,
      nextAfterIndex: null,
      reset: false,
      limit: 3,
      userQueryLimit: null,
    },
  };

  assert.equal(committedTranscriptMessages(transcript).length, 2);
  assert.equal(transcriptCommittedAfterCursor(transcript), 1);
  assert.equal(
    streamResumeCursor({
      afterCursor: transcriptCommittedAfterCursor(transcript),
      fallbackMaxIndex: null,
    }),
    2,
  );
  const cacheTranscript = transcriptForCommittedCache(transcript);
  assert.equal(cacheTranscript.messages.length, 2);
  assert.equal(cacheTranscript.pageInfo.endIndex, 2);
});

test("terminal control clears matching activeRun before cache and activity state", () => {
  const threadId = "thread::fixture-terminal-active-run";
  const runId = "run::terminal-active";
  const transcript = {
    threadId,
    remoteFound: true,
    messages: [
      controlMessage(0, {
        type: "run_start",
        threadId,
        runId,
      }),
      contentMessage(1, threadId, {
        message: { role: "assistant", content: "done", text: "done" },
      }),
      controlMessage(2, {
        type: "run_complete",
        threadId,
        runId,
      }),
    ],
    pendingInputs: [],
    threadInfo: {
      activeRun: { runId },
      channelBindings: [],
    },
    pageInfo: {
      totalMessages: 3,
      committedMessages: 3,
      returnedMessages: 3,
      startIndex: 0,
      endIndex: 3,
      hasMoreBefore: false,
      nextBeforeIndex: null,
      hasMoreAfter: false,
      nextAfterIndex: null,
      reset: false,
      limit: 3,
      userQueryLimit: null,
    },
  };

  const resolved = transcriptWithResolvedActiveRun(transcript);
  assert.equal(resolved.threadInfo.activeRun, null);
  assert.equal(transcriptForCommittedCache(resolved).threadInfo.activeRun, null);
});

test("terminal control does not clear a different activeRun", () => {
  const threadId = "thread::fixture-different-active-run";
  const transcript = {
    threadId,
    remoteFound: true,
    messages: [
      controlMessage(0, {
        type: "run_complete",
        threadId,
        runId: "run::old",
      }),
      controlMessage(1, {
        type: "run_start",
        threadId,
        runId: "run::new",
      }),
    ],
    pendingInputs: [],
    threadInfo: {
      activeRun: { runId: "run::new" },
      channelBindings: [],
    },
    pageInfo: {
      totalMessages: 2,
      committedMessages: 2,
      returnedMessages: 2,
      startIndex: 0,
      endIndex: 2,
      hasMoreBefore: false,
      nextBeforeIndex: null,
      hasMoreAfter: false,
      nextAfterIndex: null,
      reset: false,
      limit: 2,
      userQueryLimit: null,
    },
  };

  const resolved = transcriptWithResolvedActiveRun(transcript);
  assert.equal(resolved.threadInfo.activeRun.runId, "run::new");
});

test("terminal control still clears activeRun when followed by non-terminal control", () => {
  const threadId = "thread::fixture-terminal-then-title";
  const runId = "run::terminal-then-title";
  const transcript = {
    threadId,
    remoteFound: true,
    messages: [
      controlMessage(0, {
        type: "run_start",
        threadId,
        runId,
      }),
      controlMessage(1, {
        type: "run_complete",
        threadId,
        runId,
      }),
      controlMessage(2, {
        type: "thread_title_updated",
        threadId,
        runId,
        title: "Completed title",
      }),
    ],
    pendingInputs: [],
    threadInfo: {
      activeRun: { runId },
      channelBindings: [],
    },
    pageInfo: {
      totalMessages: 3,
      committedMessages: 3,
      returnedMessages: 3,
      startIndex: 0,
      endIndex: 3,
      hasMoreBefore: false,
      nextBeforeIndex: null,
      hasMoreAfter: false,
      nextAfterIndex: null,
      reset: false,
      limit: 3,
      userQueryLimit: null,
    },
  };

  const resolved = transcriptWithResolvedActiveRun(transcript);
  assert.equal(resolved.threadInfo.activeRun, null);
});

test("rewrite refetch restarts stream only for the current selected generation", () => {
  assert.equal(
    shouldRestartSelectedThreadStreamAfterRefetch({
      threadId: "thread::selected",
      selectedThreadId: "thread::selected",
      startSelectionGeneration: 7,
      currentSelectionGeneration: 7,
    }),
    true,
  );
  assert.equal(
    shouldRestartSelectedThreadStreamAfterRefetch({
      threadId: "thread::background",
      selectedThreadId: "thread::selected",
      startSelectionGeneration: 7,
      currentSelectionGeneration: 7,
    }),
    false,
  );
  assert.equal(
    shouldRestartSelectedThreadStreamAfterRefetch({
      threadId: "thread::selected",
      selectedThreadId: "thread::selected",
      startSelectionGeneration: 7,
      currentSelectionGeneration: 8,
    }),
    false,
  );
});

test("stale forward catch-up falls back to authoritative fetch at page limit", () => {
  assert.equal(
    shouldRefetchAuthoritativeAfterForwardPageLimit({
      pagesFetched: 49,
      maxPages: 50,
      hasMoreAfter: true,
    }),
    false,
  );
  assert.equal(
    shouldRefetchAuthoritativeAfterForwardPageLimit({
      pagesFetched: 50,
      maxPages: 50,
      hasMoreAfter: true,
    }),
    true,
  );
  assert.equal(
    shouldRefetchAuthoritativeAfterForwardPageLimit({
      pagesFetched: 50,
      maxPages: 50,
      hasMoreAfter: false,
    }),
    false,
  );
});

test("thread stream gap errors are recoverable sync gaps", () => {
  assert.equal(
    isThreadStreamGapError({
      runId: "thread-stream-gap",
      error: "anything",
    }),
    true,
  );
  assert.equal(
    isThreadStreamGapError({
      runId: "run::ordinary",
      error: "Thread stream seq gap after 4; authoritative refetch required",
    }),
    true,
  );
  assert.equal(
    isThreadStreamGapError({
      runId: "run::ordinary",
      error: "provider failed",
    }),
    false,
  );
});

test("forward transcript merge dedups by history index and keeps fetched copy", () => {
  const records = readJsonl("transcript-with-tool.jsonl");
  const threadId = "thread::fixture-stream-sync-tool";
  const base = {
    threadId,
    remoteFound: true,
    messages: records
      .slice(0, 3)
      .map((record, index) => contentMessage(index, threadId, record)),
    pendingInputs: [],
    pageInfo: {
      totalMessages: 5,
      returnedMessages: 3,
      startIndex: 0,
      endIndex: 3,
      hasMoreBefore: false,
      nextBeforeIndex: null,
      hasMoreAfter: true,
      nextAfterIndex: 2,
      reset: false,
      limit: 3,
      userQueryLimit: null,
    },
  };
  const page = {
    threadId,
    remoteFound: true,
    messages: records
      .slice(2)
      .map((record, offset) =>
        contentMessage(offset + 2, threadId, {
          ...record,
          message:
            offset === 0
              ? { ...record.message, text: "Fetched overwrite" }
              : record.message,
        }),
      ),
    pendingInputs: [],
    pageInfo: {
      totalMessages: 5,
      returnedMessages: 3,
      startIndex: 2,
      endIndex: 5,
      hasMoreBefore: true,
      nextBeforeIndex: 2,
      hasMoreAfter: false,
      nextAfterIndex: null,
      reset: false,
      limit: 3,
      userQueryLimit: null,
    },
  };
  const merged = mergeForwardTranscriptPage(base, page);
  assert.equal(merged.messages.length, 5);
  assert.equal(merged.messages[2].text, "Fetched overwrite");
  assert.equal(transcriptAfterCursor(merged.messages), 4);
  assert.equal(merged.pageInfo.hasMoreBefore, false);
  assert.equal(merged.pageInfo.hasMoreAfter, false);
});
