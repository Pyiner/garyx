import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  isControlTranscriptMessage,
  isToolRole,
  mergeForwardTranscriptPage,
  toolMessagesEquivalent,
  transcriptRewriteAction,
  transcriptWithResolvedActiveRun,
} from "../../../shared/transcript-sync.ts";
import { extractImageGenerationImageContent } from "../app-shell/image-generation-content.ts";
import { isRunLoadingPlaceholderMessage } from "../app-shell/loading-labels.ts";
import { ThreadTranscriptCache } from "./transcript-cache.ts";
import {
  jsonValuesEqual,
  materializeRemoteTranscript,
} from "./transcript-materialize.ts";

const here = path.dirname(fileURLToPath(import.meta.url));
const fixtureRoot = path.resolve(here, "../../../../../../test-fixtures");
const GENERATED_IMAGE_TOOL_USE_METADATA_KEY = "generated_image_tool_use_id";

function legacyAsRecord(value) {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value
    : null;
}

function legacyMetadataString(metadata, key) {
  const value = metadata?.[key];
  return typeof value === "string" ? value.trim() : "";
}

function legacyMessageOriginId(message) {
  if (message.role !== "user") {
    return "";
  }
  if (message.id.startsWith("origin:")) {
    return message.id.slice("origin:".length).trim();
  }
  return legacyMetadataString(message.metadata, "origin_id");
}

function legacyNormalizeTranscriptMessageId(message) {
  const originId = legacyMessageOriginId(message);
  if (!originId) {
    return message;
  }
  const id = `origin:${originId}`;
  return message.id === id ? message : { ...message, id };
}

function legacyJsonValuesEqual(left, right) {
  return JSON.stringify(left ?? null) === JSON.stringify(right ?? null);
}

function legacyCanReuse(existing, remote, options = {}) {
  return (
    existing.localState === "remote_final" &&
    existing.role === remote.role &&
    existing.text === remote.text &&
    legacyJsonValuesEqual(existing.content, remote.content) &&
    (options.ignoreTimestamp || existing.timestamp === remote.timestamp) &&
    existing.toolUseId === remote.toolUseId &&
    existing.toolName === remote.toolName &&
    existing.isError === remote.isError &&
    legacyJsonValuesEqual(existing.metadata, remote.metadata) &&
    existing.kind === remote.kind &&
    existing.internal === remote.internal &&
    existing.internalKind === remote.internalKind &&
    existing.loopOrigin === remote.loopOrigin &&
    existing.pending !== true &&
    existing.error === remote.error
  );
}

// Frozen pre-change materializer. It intentionally retains findIndex and
// JSON.stringify and must never import optimized matching helpers.
function legacyMaterializeRemoteTranscript(transcript, existing, options = {}) {
  const usedExistingIndexes = new Set();

  const materializeMessage = (message) => {
    const matchedIndex = existing.findIndex(
      (entry, index) =>
        !usedExistingIndexes.has(index) && entry.id === message.id,
    );
    const matchedEntry = matchedIndex >= 0 ? existing[matchedIndex] : null;
    if (matchedIndex >= 0) {
      usedExistingIndexes.add(matchedIndex);
    }
    if (
      matchedEntry &&
      legacyCanReuse(matchedEntry, message, {
        ignoreTimestamp: options.ignoreTimestampForStableMessages,
      })
    ) {
      return matchedEntry.seq === message.seq
        ? matchedEntry
        : { ...matchedEntry, seq: message.seq ?? matchedEntry.seq };
    }
    return {
      ...message,
      id: matchedEntry?.id || message.id,
      intentId: matchedEntry?.intentId,
      remoteRunId: matchedEntry?.remoteRunId,
      localState: "remote_final",
      pending: false,
      error: message.error,
    };
  };

  const materializeGeneratedImageMessage = (sourceMessage, content) => {
    const toolUseId = sourceMessage.toolUseId?.trim() || "";
    const synthetic = {
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
    let matchedIndex = existing.findIndex(
      (entry, index) =>
        !usedExistingIndexes.has(index) && entry.id === synthetic.id,
    );
    if (matchedIndex < 0 && toolUseId) {
      matchedIndex = existing.findIndex((entry, index) => {
        const metadata = legacyAsRecord(entry.metadata);
        return (
          !usedExistingIndexes.has(index) &&
          entry.role === "assistant" &&
          metadata?.[GENERATED_IMAGE_TOOL_USE_METADATA_KEY] === toolUseId
        );
      });
    }
    if (matchedIndex < 0) {
      const contentSignature = JSON.stringify(content);
      matchedIndex = existing.findIndex(
        (entry, index) =>
          !usedExistingIndexes.has(index) &&
          entry.role === "assistant" &&
          !entry.text.trim() &&
          JSON.stringify(entry.content) === contentSignature,
      );
    }
    const matchedEntry = matchedIndex >= 0 ? existing[matchedIndex] : null;
    if (matchedIndex >= 0) {
      usedExistingIndexes.add(matchedIndex);
    }
    if (
      matchedEntry &&
      legacyCanReuse(matchedEntry, synthetic, {
        ignoreTimestamp: options.ignoreTimestampForStableMessages,
      })
    ) {
      return matchedEntry;
    }
    return {
      ...synthetic,
      id: matchedEntry?.id || synthetic.id,
      intentId: matchedEntry?.intentId,
      remoteRunId: matchedEntry?.remoteRunId,
      localState: "remote_final",
      pending: false,
      error: false,
    };
  };

  const materialized = [];
  for (const message of transcript) {
    if (
      isControlTranscriptMessage(message) ||
      isRunLoadingPlaceholderMessage(message)
    ) {
      continue;
    }
    materialized.push(
      materializeMessage(legacyNormalizeTranscriptMessageId(message)),
    );
    if (message.role === "tool_result") {
      const imageContent = extractImageGenerationImageContent(message);
      if (imageContent) {
        materialized.push(
          materializeGeneratedImageMessage(message, imageContent),
        );
      }
    }
  }
  return materialized;
}

function legacyHistoryIndex(message) {
  if (message.localState !== "remote_final") {
    return null;
  }
  if (typeof message.seq === "number" && Number.isFinite(message.seq)) {
    return Math.max(0, message.seq - 1);
  }
  const suffix = message.id.split(":").pop();
  return suffix && /^\d+$/.test(suffix) ? Number(suffix) : null;
}

function legacyEarliestRemoteIndex(messages) {
  let earliest = null;
  for (const message of messages) {
    const index = legacyHistoryIndex(message);
    if (index !== null && (earliest === null || index < earliest)) {
      earliest = index;
    }
  }
  return earliest;
}

function legacyPaginationFromTranscript(transcript, loadingBefore = false) {
  return {
    hasMoreBefore: Boolean(transcript.pageInfo?.hasMoreBefore),
    nextBeforeIndex:
      typeof transcript.pageInfo?.nextBeforeIndex === "number"
        ? transcript.pageInfo.nextBeforeIndex
        : null,
    loadingBefore,
  };
}

function legacyMergePagination(current, incoming, existing) {
  if (!current) {
    return incoming;
  }
  if (!current.hasMoreBefore) {
    const earliest = legacyEarliestRemoteIndex(existing);
    if (earliest === 0 || !incoming.hasMoreBefore) {
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

function legacyVisible(messages) {
  return messages.filter((message) => !isControlTranscriptMessage(message));
}

function legacyResolveIntentHistoryMatch(intent, messages) {
  let userIndex = -1;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (legacyMessageOriginId(messages[index]) === intent.intentId) {
      userIndex = index;
      break;
    }
  }
  if (userIndex < 0) {
    return { userVisible: false, assistantVisible: false };
  }
  const followUp = messages.slice(userIndex + 1);
  const assistant = followUp.filter((message) => message.role === "assistant");
  const expected = intent.responseText?.trim() || "";
  return {
    userVisible: true,
    assistantVisible: expected
      ? assistant.some((message) => message.text.trim() === expected)
      : assistant.length > 0 || followUp.some((message) => isToolRole(message.role)),
  };
}

function legacyMergeRemoteWithLocal(transcript, existing, options) {
  const visible = legacyVisible(transcript);
  if (visible.length === 0) {
    return existing.length > 0 ? existing : [];
  }
  const remote = legacyMaterializeRemoteTranscript(visible, existing, {
    ignoreTimestampForStableMessages: options.activeRunLiveRows,
  });
  const remoteIds = new Set(remote.map((entry) => entry.id));
  const preservedBefore = [];
  const preservedLocal = existing.filter((entry, index, entries) => {
    if (entry.localState === "remote_final") {
      const historyIndex = legacyHistoryIndex(entry);
      if (
        typeof options.preserveRemoteBeforeIndex === "number" &&
        historyIndex !== null &&
        historyIndex < options.preserveRemoteBeforeIndex &&
        !remoteIds.has(entry.id)
      ) {
        preservedBefore.push(entry);
      }
      return false;
    }
    if (entries.findIndex((candidate) => candidate.id === entry.id) !== index) {
      return false;
    }
    if (!entry.intentId) {
      return entry.localState === "error" || entry.localState === "interrupted";
    }
    const intent = options.intentForId(entry.intentId);
    if (!intent) {
      return entry.localState === "error" || entry.localState === "interrupted";
    }
    if (entry.role === "user") {
      return !(
        remoteIds.has(entry.id) || remoteIds.has(`origin:${intent.intentId}`)
      );
    }
    const match = legacyResolveIntentHistoryMatch(intent, visible);
    if (entry.role === "assistant") {
      return !match.assistantVisible;
    }
    if (isToolRole(entry.role)) {
      return (
        options.threadRunActive !== false &&
        !remote.some((candidate) => toolMessagesEquivalent(candidate, entry))
      );
    }
    return false;
  });
  return [...preservedBefore, ...remote, ...preservedLocal];
}

function legacyCommittedForwardPage(base, event) {
  const resolvedBase = base || {
    threadId: event.threadId,
    remoteFound: true,
    messages: [],
    pendingInputs: [],
    pageInfo: null,
  };
  return mergeForwardTranscriptPage(resolvedBase, {
    threadId: event.threadId,
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
        limit: 100,
        userQueryLimit: 10,
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

function emptyLegacyState() {
  return {
    snapshot: null,
    messages: [],
    pagination: null,
    threadInfo: null,
    pendingInputs: [],
  };
}

function legacyApplyRemote(state, transcript, intentForId = () => null) {
  const resolved = transcriptWithResolvedActiveRun(transcript);
  const existing = [...state.messages];
  const pagination = legacyMergePagination(
    state.pagination,
    legacyPaginationFromTranscript(resolved),
    existing,
  );
  const visible = legacyVisible(resolved.messages);
  const messages = legacyMergeRemoteWithLocal(visible, existing, {
    activeRunLiveRows: Boolean(resolved.threadInfo?.activeRun),
    preserveRemoteBeforeIndex: resolved.pageInfo?.startIndex ?? null,
    threadRunActive: Boolean(resolved.threadInfo?.activeRun),
    intentForId,
  });
  return {
    snapshot: resolved,
    messages,
    pagination,
    threadInfo: resolved.threadInfo ?? null,
    pendingInputs: resolved.pendingInputs ?? [],
  };
}

function legacyApplyCommitted(state, event, intentForId = () => null) {
  if (transcriptRewriteAction(event.message) === "refetch_authoritative") {
    return { state, outcome: "refetch_authoritative" };
  }
  return {
    state: legacyApplyRemote(
      state,
      legacyCommittedForwardPage(state.snapshot, event),
      intentForId,
    ),
    outcome: "applied",
  };
}

function assertCacheEqualsLegacy(cache, legacy, label) {
  assert.deepEqual(cache.getSnapshotTranscript(), legacy.snapshot, `${label}: snapshot`);
  assert.deepEqual(cache.getUiMessages(), legacy.messages, `${label}: messages`);
  assert.deepEqual(cache.getHistoryPagination(), legacy.pagination, `${label}: pagination`);
  assert.deepEqual(cache.getThreadInfo(), legacy.threadInfo, `${label}: threadInfo`);
  assert.deepEqual(
    cache.getPendingRemoteInputs(),
    legacy.pendingInputs,
    `${label}: pending inputs`,
  );
}

function pageInfo(count) {
  return {
    totalMessages: count,
    committedMessages: count,
    returnedMessages: count,
    startIndex: 0,
    endIndex: count,
    hasMoreBefore: false,
    nextBeforeIndex: null,
    hasMoreAfter: false,
    nextAfterIndex: null,
    limit: 100,
    userQueryLimit: 10,
  };
}

function wireMessage(threadId, seq, role = "assistant", overrides = {}) {
  return {
    id: `${threadId}:${seq - 1}`,
    seq,
    role,
    text: `fixture message ${seq}`,
    content: { type: "text", text: `payload-${seq}` },
    metadata: { source: "fixture", seq },
    timestamp: `2026-01-01T00:${String(seq % 60).padStart(2, "0")}:00Z`,
    ...overrides,
  };
}

function eventForMessage(threadId, message, runId = "run::fixture") {
  return {
    type: "committed_message",
    threadId,
    runId,
    seq: message.seq,
    message,
  };
}

function mapCapturedRecord(record, fallbackThreadId) {
  const seq = Number(record.seq);
  const threadId = record.thread_id || record.threadId || fallbackThreadId;
  const runId = record.run_id || record.runId || "run::captured";
  const raw = legacyAsRecord(record.message) || {};
  const allowedRole = ["assistant", "user", "tool", "tool_use", "tool_result"];
  const role = allowedRole.includes(raw.role) ? raw.role : "system";
  const metadata = legacyAsRecord(raw.metadata);
  const explicitKind = typeof raw.kind === "string" ? raw.kind.trim() : "";
  const kind = explicitKind ||
    (raw.internal_kind === "control"
      ? "control"
      : isToolRole(role)
        ? "tool_trace"
        : role === "assistant"
          ? "assistant_reply"
          : role === "user"
            ? "user_input"
            : undefined);
  const isControl = kind === "control" || raw.internal_kind === "control";
  const contentRecord = legacyAsRecord(raw.content);
  const message = {
    id: `${threadId}:${seq - 1}`,
    seq,
    role: isControl ? "system" : role,
    text: isControl
      ? ""
      : typeof raw.text === "string" && raw.text.trim()
        ? raw.text
        : typeof raw.content === "string"
          ? raw.content.trim()
          : "",
    content: isControl ? raw : raw.content,
    input: raw.input,
    result: raw.result,
    timestamp: raw.timestamp || record.timestamp || null,
    toolUseId: raw.tool_use_id || raw.toolUseId || null,
    toolName:
      raw.tool_name ||
      raw.toolName ||
      metadata?.item_type ||
      metadata?.itemType ||
      contentRecord?.type ||
      null,
    isError: raw.is_error ?? raw.isError,
    metadata: metadata && Object.keys(metadata).length > 0 ? metadata : null,
    kind: isControl ? "control" : kind,
    internal: isControl || Boolean(raw.internal),
    internalKind:
      raw.internal_kind || raw.internalKind || (isControl ? "control" : null),
    loopOrigin: raw.loop_origin || raw.loopOrigin || null,
  };
  return { type: "committed_message", threadId, runId, seq, message };
}

function readJsonl(relativePath) {
  return readFileSync(path.join(fixtureRoot, relativePath), "utf8")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

function compareCommittedSequence(name, events) {
  const cache = new ThreadTranscriptCache();
  let legacy = emptyLegacyState();
  for (const [index, event] of events.entries()) {
    const legacyResult = legacyApplyCommitted(legacy, event);
    const outcome = cache.applyCommittedMessage(event, { intentForId: () => null });
    assert.equal(outcome, legacyResult.outcome, `${name} prefix ${index + 1}: outcome`);
    legacy = legacyResult.state;
    assertCacheEqualsLegacy(cache, legacy, `${name} prefix ${index + 1}`);
  }
}

test("JSON-safe comparator matches the frozen stringify relation", () => {
  const hole = [];
  hole.length = 1;
  const pairs = [
    [null, undefined],
    [0, -0],
    [null, Number.NaN],
    [null, Number.POSITIVE_INFINITY],
    [[undefined], [null]],
    [[() => true, Symbol("ignored")], [null, null]],
    [hole, [null]],
    [{ a: 1, ignored: undefined }, { a: 1 }],
    [{ a: 1, ignored: () => true }, { a: 1 }],
    [{ a: 1, b: [2, null] }, { a: 1, b: [2, null] }],
    [{ a: 1, b: 2 }, { b: 2, a: 1 }],
    [[1, 2], [2, 1]],
  ];
  for (const [index, [left, right]] of pairs.entries()) {
    assert.equal(
      jsonValuesEqual(left, right),
      legacyJsonValuesEqual(left, right),
      `comparator case ${index}`,
    );
  }

  const cycle = {};
  cycle.self = cycle;
  assert.equal(jsonValuesEqual(cycle, cycle), false);
  assert.equal(jsonValuesEqual(1n, 1n), false);
  assert.equal(jsonValuesEqual(new Date(0), new Date(0)), false);
});

test("linear indexes retain duplicate-id and generated-image first-unused matching", () => {
  const duplicateExisting = [
    {
      id: "duplicate",
      role: "assistant",
      text: "first",
      content: { value: 1 },
      metadata: null,
      localState: "remote_final",
      pending: false,
      error: undefined,
    },
    {
      id: "duplicate",
      role: "assistant",
      text: "second",
      content: { value: 2 },
      metadata: null,
      localState: "remote_final",
      pending: false,
      error: undefined,
    },
  ];
  const duplicateRemote = [
    { id: "duplicate", role: "assistant", text: "first", content: { value: 1 }, metadata: null },
    { id: "duplicate", role: "assistant", text: "second", content: { value: 2 }, metadata: null },
  ];
  const legacyDuplicates = legacyMaterializeRemoteTranscript(
    duplicateRemote,
    duplicateExisting,
  );
  const nextDuplicates = materializeRemoteTranscript(
    duplicateRemote,
    duplicateExisting,
  );
  assert.deepEqual(nextDuplicates, legacyDuplicates);
  assert.equal(nextDuplicates[0], duplicateExisting[0]);
  assert.equal(nextDuplicates[1], duplicateExisting[1]);

  const source = {
    id: "thread::generated-index:0",
    seq: 1,
    role: "tool_result",
    text: "",
    toolUseId: "call-generated-index",
    toolName: "imageGeneration",
    metadata: { item_type: "imageGeneration" },
    content: {
      type: "imageGeneration",
      savedPath: "/Users/test/generated-index.png",
    },
  };
  const first = legacyMaterializeRemoteTranscript([source], []);
  const movedSynthetic = {
    ...first[1],
    id: "legacy-generated-image-id",
  };
  const existing = [first[0], movedSynthetic];
  const legacyGenerated = legacyMaterializeRemoteTranscript([source], existing);
  const nextGenerated = materializeRemoteTranscript([source], existing);
  assert.deepEqual(nextGenerated, legacyGenerated);
  assert.equal(
    nextGenerated[1].id,
    movedSynthetic.id,
    "tool-use fallback carries the first unused row id",
  );

  const whitespaceToolIdCandidate = {
    ...movedSynthetic,
    id: "whitespace-tool-id-candidate",
    content: [{ type: "image", path: "/Users/test/not-the-image.png" }],
    metadata: {
      ...movedSynthetic.metadata,
      [GENERATED_IMAGE_TOOL_USE_METADATA_KEY]: " call-generated-index ",
    },
  };
  const contentCandidate = {
    ...movedSynthetic,
    id: "content-candidate",
    metadata: { source: "legacy" },
  };
  const fallbackExisting = [
    first[0],
    whitespaceToolIdCandidate,
    contentCandidate,
  ];
  const legacyContentFallback = legacyMaterializeRemoteTranscript(
    [source],
    fallbackExisting,
  );
  const nextContentFallback = materializeRemoteTranscript(
    [source],
    fallbackExisting,
  );
  assert.deepEqual(nextContentFallback, legacyContentFallback);
  assert.equal(nextContentFallback[1].id, contentCandidate.id);
});

test("1200-message committed stream is structurally equal, reference-stable, and always incremental", () => {
  const threadId = "thread::large-incremental-oracle";
  const seedMessages = Array.from({ length: 1_200 }, (_, index) =>
    wireMessage(threadId, index + 1, index % 3 === 0 ? "user" : "assistant"),
  );
  const transcript = {
    threadId,
    remoteFound: true,
    messages: seedMessages,
    pendingInputs: [],
    threadInfo: null,
    pageInfo: pageInfo(seedMessages.length),
  };
  const cache = new ThreadTranscriptCache();
  cache.applyRemote(transcript, { intentForId: () => null });
  let legacy = legacyApplyRemote(emptyLegacyState(), transcript);
  assertCacheEqualsLegacy(cache, legacy, "large seed");
  const stableEntries = [...cache.getUiMessages()];
  let optimizedStringifyCalls = 0;

  for (let offset = 1; offset <= 120; offset += 1) {
    const seq = seedMessages.length + offset;
    const event = eventForMessage(
      threadId,
      wireMessage(threadId, seq, seq % 2 === 0 ? "assistant" : "user"),
    );
    const originalStringify = JSON.stringify;
    JSON.stringify = (...args) => {
      optimizedStringifyCalls += 1;
      return originalStringify(...args);
    };
    let outcome;
    try {
      outcome = cache.applyCommittedMessage(event, { intentForId: () => null });
    } finally {
      JSON.stringify = originalStringify;
    }
    const legacyResult = legacyApplyCommitted(legacy, event);
    legacy = legacyResult.state;
    assert.equal(outcome, legacyResult.outcome);
    assertCacheEqualsLegacy(cache, legacy, `large prefix ${offset}`);
  }

  assert.deepEqual(cache.getCommittedApplyStats(), {
    incremental: 120,
    fullFallback: 0,
  });
  assert.equal(optimizedStringifyCalls, 0);
  stableEntries.forEach((entry, index) => {
    assert.equal(cache.getUiMessages()[index], entry, `stable entry ${index}`);
  });
});

test("all captured streams and render-state fixture sources match the frozen fold per prefix", () => {
  const streamDirectory = path.join(fixtureRoot, "stream-sync");
  for (const fixtureName of readdirSync(streamDirectory)
    .filter((name) => name.endsWith(".jsonl"))
    .sort()) {
    const records = readJsonl(path.join("stream-sync", fixtureName));
    const events = records.map((record) =>
      mapCapturedRecord(record, `thread::captured-${fixtureName}`),
    );
    compareCommittedSequence(`stream-sync/${fixtureName}`, events);
  }

  const renderCases = JSON.parse(
    readFileSync(path.join(fixtureRoot, "render-layer/render-state-cases.json"), "utf8"),
  ).cases;
  for (const reducerCase of renderCases) {
    let records = reducerCase.records || [];
    if (reducerCase.source?.fixture) {
      records = readJsonl(reducerCase.source.fixture).filter((record) => {
        const seq = Number(record.seq);
        return (
          (reducerCase.source.min_seq == null || seq >= reducerCase.source.min_seq) &&
          (reducerCase.source.max_seq == null || seq <= reducerCase.source.max_seq)
        );
      });
    }
    const events = records.map((record) =>
      mapCapturedRecord(record, `thread::render-case-${reducerCase.name}`),
    );
    compareCommittedSequence(`render case ${reducerCase.name}`, events);
  }
});

test("duplicate origin, optimistic echo, loading, generated image, and rewrite use exact legacy fallbacks", () => {
  const threadId = "thread::incremental-edge-cases";
  const intent = {
    intentId: "intent-edge",
    threadId,
    state: "awaiting_response",
    dispatchMode: "sync_send",
    responseText: "",
  };
  const intentForId = (id) => (id === intent.intentId ? intent : null);
  const seed = {
    threadId,
    remoteFound: true,
    messages: [wireMessage(threadId, 1, "assistant")],
    pendingInputs: [],
    threadInfo: null,
    pageInfo: pageInfo(1),
  };
  const cache = new ThreadTranscriptCache();
  cache.applyRemote(seed, { intentForId });
  let legacy = legacyApplyRemote(emptyLegacyState(), seed, intentForId);
  const stablePrefix = cache.getUiMessages()[0];
  const optimistic = {
    id: `origin:${intent.intentId}`,
    role: "user",
    text: "optimistic",
    localState: "optimistic",
    pending: true,
    intentId: intent.intentId,
  };
  cache.setUiMessages([...cache.getUiMessages(), optimistic]);
  legacy = { ...legacy, messages: [...legacy.messages, optimistic] };

  const echo = eventForMessage(
    threadId,
    wireMessage(threadId, 2, "user", {
      text: "optimistic",
      metadata: { origin_id: intent.intentId },
    }),
  );
  let result = legacyApplyCommitted(legacy, echo, intentForId);
  assert.equal(cache.applyCommittedMessage(echo, { intentForId }), result.outcome);
  legacy = result.state;
  assertCacheEqualsLegacy(cache, legacy, "optimistic echo");
  assert.equal(cache.getUiMessages()[0], stablePrefix);
  assert.equal(cache.getCommittedApplyStats().incremental, 1);

  const duplicateOrigin = eventForMessage(
    threadId,
    wireMessage(threadId, 3, "user", {
      metadata: { origin_id: intent.intentId },
    }),
  );
  result = legacyApplyCommitted(legacy, duplicateOrigin, intentForId);
  cache.applyCommittedMessage(duplicateOrigin, { intentForId });
  legacy = result.state;
  assertCacheEqualsLegacy(cache, legacy, "duplicate origin fallback");

  const loading = eventForMessage(
    threadId,
    wireMessage(threadId, 4, "assistant", {
      text: "Garyx is working through the run…",
    }),
  );
  result = legacyApplyCommitted(legacy, loading, intentForId);
  const beforeLoadingArray = cache.getUiMessages();
  cache.applyCommittedMessage(loading, { intentForId });
  legacy = result.state;
  assertCacheEqualsLegacy(cache, legacy, "loading fallback");
  assert.equal(cache.getUiMessages(), beforeLoadingArray);

  const generatedImage = eventForMessage(
    threadId,
    wireMessage(threadId, 5, "tool_result", {
      text: "",
      toolUseId: "call-image-edge",
      toolName: "imageGeneration",
      metadata: { item_type: "imageGeneration" },
      content: {
        type: "imageGeneration",
        savedPath: "/Users/test/generated-edge.png",
      },
    }),
  );
  result = legacyApplyCommitted(legacy, generatedImage, intentForId);
  cache.applyCommittedMessage(generatedImage, { intentForId });
  legacy = result.state;
  assertCacheEqualsLegacy(cache, legacy, "generated image fallback");
  assert.ok(
    cache.getUiMessages().some((entry) => entry.id.startsWith("generated-image:")),
  );

  const rewrite = eventForMessage(
    threadId,
    wireMessage(threadId, 6, "system", {
      text: "",
      kind: "control",
      internal: true,
      internalKind: "control",
      content: { control: { kind: "range_rewrite", from_seq: 2, to_seq: 4 } },
    }),
  );
  const beforeRewrite = cache.getSnapshotTranscript();
  result = legacyApplyCommitted(legacy, rewrite, intentForId);
  assert.equal(cache.applyCommittedMessage(rewrite, { intentForId }), result.outcome);
  assert.equal(cache.getSnapshotTranscript(), beforeRewrite);
  assert.deepEqual(cache.getCommittedApplyStats(), {
    incremental: 1,
    fullFallback: 3,
  });
});

test("local tool preservation compares against the stable prefix plus appended tail", () => {
  const threadId = "thread::local-tool-candidates";
  const intent = {
    intentId: "intent-tool",
    threadId,
    state: "awaiting_response",
    dispatchMode: "sync_send",
    responseText: "",
  };
  const intentForId = (id) => (id === intent.intentId ? intent : null);
  const remoteTool = wireMessage(threadId, 2, "tool_result", {
    toolUseId: "tool-existing",
    toolName: "Read",
    text: "done",
  });
  const seed = {
    threadId,
    remoteFound: true,
    messages: [
      wireMessage(threadId, 1, "user", {
        metadata: { origin_id: intent.intentId },
      }),
      remoteTool,
    ],
    pendingInputs: [],
    threadInfo: { activeRun: { runId: "run-tool" } },
    pageInfo: pageInfo(2),
  };
  const duplicateLocalTool = {
    id: "local-tool-existing",
    role: "tool_result",
    text: "done",
    toolUseId: "tool-existing",
    toolName: "Read",
    localState: "remote_partial",
    intentId: intent.intentId,
  };
  const cache = new ThreadTranscriptCache();
  cache.applyRemote(seed, { intentForId });
  cache.setUiMessages([...cache.getUiMessages(), duplicateLocalTool]);
  let legacy = legacyApplyRemote(emptyLegacyState(), seed, intentForId);
  legacy = { ...legacy, messages: [...legacy.messages, duplicateLocalTool] };

  const append = eventForMessage(
    threadId,
    wireMessage(threadId, 3, "assistant", { text: "tail" }),
    "run-tool",
  );
  const result = legacyApplyCommitted(legacy, append, intentForId);
  cache.applyCommittedMessage(append, { intentForId });
  assertCacheEqualsLegacy(cache, result.state, "full remote tool candidate pool");
  assert.ok(!cache.getUiMessages().some((entry) => entry === duplicateLocalTool));
  assert.equal(cache.getCommittedApplyStats().incremental, 1);
});

test("older-page prefix survives a windowed incremental tail append", () => {
  const threadId = "thread::windowed-pagination";
  const current = {
    threadId,
    remoteFound: true,
    messages: [
      wireMessage(threadId, 3, "user"),
      wireMessage(threadId, 4, "assistant"),
    ],
    pendingInputs: [],
    threadInfo: null,
    pageInfo: {
      ...pageInfo(4),
      returnedMessages: 2,
      startIndex: 2,
      hasMoreBefore: true,
      nextBeforeIndex: 2,
    },
  };
  const older = {
    threadId,
    remoteFound: true,
    messages: [
      wireMessage(threadId, 1, "user"),
      wireMessage(threadId, 2, "assistant"),
    ],
    pendingInputs: [],
    threadInfo: null,
    pageInfo: {
      ...pageInfo(4),
      returnedMessages: 2,
      startIndex: 0,
      endIndex: 2,
      hasMoreBefore: false,
      nextBeforeIndex: null,
    },
  };
  const cache = new ThreadTranscriptCache();
  cache.applyRemote(current, { intentForId: () => null });
  let legacy = legacyApplyRemote(emptyLegacyState(), current);
  cache.applyOlderPage(older);
  const olderEntries = legacyMaterializeRemoteTranscript(
    legacyVisible(older.messages),
    [],
  );
  legacy = {
    ...legacy,
    messages: [...olderEntries, ...legacy.messages],
    pagination: legacyPaginationFromTranscript(older),
  };
  assertCacheEqualsLegacy(cache, legacy, "older prepend");
  const olderReferences = cache.getUiMessages().slice(0, 2);

  const append = eventForMessage(
    threadId,
    wireMessage(threadId, 5, "assistant"),
  );
  const result = legacyApplyCommitted(legacy, append);
  cache.applyCommittedMessage(append, { intentForId: () => null });
  assertCacheEqualsLegacy(cache, result.state, "windowed append");
  assert.equal(cache.getCommittedApplyStats().incremental, 1);
  assert.equal(cache.getUiMessages()[0], olderReferences[0]);
  assert.equal(cache.getUiMessages()[1], olderReferences[1]);
});
