#!/usr/bin/env node

import { performance } from "node:perf_hooks";

import { ThreadTranscriptCache } from "../src/renderer/src/gateway-mirror/transcript-cache.ts";

const DEFAULTS = {
  seedMessages: 1_200,
  committedEvents: 120,
  payloadBytes: 2_048,
  samples: 5,
  warmups: 1,
};

function positiveIntegerArgument(name, fallback) {
  const prefix = `--${name}=`;
  const raw = process.argv.find((argument) => argument.startsWith(prefix));
  if (!raw) {
    return fallback;
  }
  const value = Number(raw.slice(prefix.length));
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error(`${name} must be a positive integer`);
  }
  return value;
}

const config = {
  seedMessages: positiveIntegerArgument(
    "seed-messages",
    DEFAULTS.seedMessages,
  ),
  committedEvents: positiveIntegerArgument(
    "committed-events",
    DEFAULTS.committedEvents,
  ),
  payloadBytes: positiveIntegerArgument("payload-bytes", DEFAULTS.payloadBytes),
  samples: positiveIntegerArgument("samples", DEFAULTS.samples),
  warmups: positiveIntegerArgument("warmups", DEFAULTS.warmups),
};

const threadId = "thread::transcript-materialization-benchmark";
const payload = "x".repeat(config.payloadBytes);

function message(seq) {
  const role = seq % 4 === 0
    ? "tool_result"
    : seq % 3 === 0
      ? "assistant"
      : "user";
  return {
    // Production stream mapping stamps the 1-based committed seq alongside
    // a transcript id whose suffix is the 0-based history index.
    id: `${threadId}:${seq - 1}`,
    seq,
    role,
    text: `Synthetic transcript message ${seq}`,
    content: {
      type: "text",
      value: `${payload}:${seq}`,
    },
    metadata: {
      fixture: "large-thread",
      ordinal: seq,
    },
    timestamp: `2026-01-01T00:${String(seq % 60).padStart(2, "0")}:00Z`,
  };
}

function pageInfo(count) {
  return {
    totalMessages: count,
    committedMessages: count,
    returnedMessages: count,
    startIndex: 1,
    endIndex: count,
    hasMoreBefore: false,
    nextBeforeIndex: null,
    hasMoreAfter: false,
    nextAfterIndex: null,
    limit: config.seedMessages,
    userQueryLimit: 10,
  };
}

function initialTranscript() {
  return {
    threadId,
    remoteFound: true,
    messages: Array.from(
      { length: config.seedMessages },
      (_, index) => message(index + 1),
    ),
    pendingInputs: [],
    threadInfo: null,
    pageInfo: pageInfo(config.seedMessages),
  };
}

function percentile(sorted, ratio) {
  return sorted[Math.floor((sorted.length - 1) * ratio)];
}

function runSample() {
  const cache = new ThreadTranscriptCache();
  cache.applyRemote(initialTranscript(), { intentForId: () => null });
  const stableSeedEntries = [...cache.getUiMessages()];

  const originalStringify = JSON.stringify;
  let stringifyCalls = 0;
  let stringifyBytes = 0;
  JSON.stringify = function instrumentedStringify(...args) {
    const result = originalStringify.apply(JSON, args);
    stringifyCalls += 1;
    stringifyBytes += typeof result === "string" ? Buffer.byteLength(result) : 0;
    return result;
  };

  const startedAt = performance.now();
  try {
    for (let offset = 1; offset <= config.committedEvents; offset += 1) {
      const seq = config.seedMessages + offset;
      cache.applyCommittedMessage(
        {
          type: "committed_message",
          runId: "run-benchmark",
          threadId,
          seq,
          message: message(seq),
        },
        { intentForId: () => null },
      );
    }
  } finally {
    JSON.stringify = originalStringify;
  }

  return {
    elapsedMs: performance.now() - startedAt,
    stringifyCalls,
    stringifyBytes,
    finalMessageCount: cache.getUiMessages().length,
    stableSeedReferences: stableSeedEntries.filter(
      (entry, index) => cache.getUiMessages()[index] === entry,
    ).length,
    committedApplyStats: cache.getCommittedApplyStats(),
  };
}

for (let index = 0; index < config.warmups; index += 1) {
  runSample();
}

const samples = Array.from({ length: config.samples }, runSample);
const elapsed = samples.map((sample) => sample.elapsedMs).sort((a, b) => a - b);
const output = {
  benchmark: "desktop committed transcript materialization",
  config,
  node: process.version,
  platform: `${process.platform}-${process.arch}`,
  result: {
    medianMs: Number(percentile(elapsed, 0.5).toFixed(2)),
    minMs: Number(elapsed[0].toFixed(2)),
    maxMs: Number(elapsed.at(-1).toFixed(2)),
    stringifyCallsPerSample: samples[0].stringifyCalls,
    stringifyBytesPerSample: samples[0].stringifyBytes,
    finalMessageCount: samples[0].finalMessageCount,
    stableSeedReferences: samples[0].stableSeedReferences,
    committedApplyStats: samples[0].committedApplyStats,
  },
  samplesMs: samples.map((sample) => Number(sample.elapsedMs.toFixed(2))),
};

console.log(JSON.stringify(output, null, 2));
