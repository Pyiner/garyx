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

function transcriptFromLifecycleFixture() {
  const events = readJsonl("stream-lifecycle.jsonl");
  const threadId = "thread::fixture-stream-sync-life";
  const messages = [];
  for (const event of events) {
    const index = messages.length;
    switch (event.type) {
      case "run_start":
      case "done":
      case "run_complete":
        messages.push(controlMessage(index, event));
        break;
      case "committed_message":
        messages.push(contentMessage(index, threadId, event));
        break;
      default:
        break;
    }
  }
  return messages;
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

test("lifecycle fixture replay reaches idle terminal state from control records", () => {
  const state = reduceTranscriptRunState(transcriptFromLifecycleFixture());
  assert.equal(state.busy, false);
  assert.equal(state.activity, "idle");
  assert.equal(state.terminalStatus, "completed");
  assert.equal(state.activeRunId, null);
});

test("incremental run-state apply matches full reducer on lifecycle fixture", () => {
  const messages = transcriptFromLifecycleFixture();
  const incremental = messages.reduce(
    (state, message, index) =>
      applyTranscriptRunStateRecord(state, message, { seq: index + 1 }),
    reduceTranscriptRunState([]),
  );
  assert.deepEqual(incremental, reduceTranscriptRunState(messages));
});

test("user_ack fixture replays ack position and reconciling activity", () => {
  const events = readJsonl("stream-events-with-user-ack.jsonl");
  const threadId = "thread::fixture-stream-sync-ack";
  const messages = [
    controlMessage(0, {
      type: "run_start",
      threadId,
      runId: "run::fixture-ack",
    }),
  ];
  for (const event of events) {
    const index = messages.length;
    if (event.type === "tool_use" || event.type === "tool_result") {
      messages.push(contentMessage(index, threadId, event));
    } else if (
      event.type === "user_ack" ||
      event.type === "assistant_boundary" ||
      event.type === "done"
    ) {
      messages.push(controlMessage(index, event));
    }
  }
  const state = reduceTranscriptRunState(messages);
  assert.equal(state.busy, true);
  assert.equal(state.activity, "reconciling");
  assert.equal(state.lastUserAckPendingInputId, "pending-fixture-followup");
  assert.equal(state.lastUserAckSeq, 3);
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

test("active-run overlay rows do not advance committed cache cursor", () => {
  const threadId = "thread::fixture-overlay";
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
      activeRun: { runId: "run::overlay" },
      channelBindings: [],
    },
    pageInfo: {
      totalMessages: 3,
      committedMessages: 2,
      overlayMessages: 1,
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
  assert.equal(cacheTranscript.pageInfo.overlayMessages, 0);
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
      overlayMessages: 0,
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
      overlayMessages: 0,
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
      overlayMessages: 0,
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
