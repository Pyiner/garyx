import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";

import { app } from "electron";

import type { ThreadTranscript } from "@shared/contracts";

const CACHE_VERSION = 1;

interface CachedThreadTranscriptFile {
  version: number;
  savedAt: string;
  transcript: ThreadTranscript;
}

const cacheMutationQueues = new Map<string, Promise<void>>();
const cacheMutationGenerations = new Map<string, number>();

function transcriptCacheDir(): string {
  return join(app.getPath("userData"), "transcript-cache");
}

function cacheFileName(threadId: string): string {
  return `${Buffer.from(threadId, "utf8").toString("base64url")}.json`;
}

function transcriptCachePath(threadId: string): string {
  return join(transcriptCacheDir(), cacheFileName(threadId));
}

async function waitForCacheMutations(threadId: string): Promise<void> {
  const queue = cacheMutationQueues.get(threadId);
  if (!queue) {
    return;
  }
  await queue.catch(() => undefined);
}

function enqueueLatestCacheMutation(
  threadId: string,
  operation: () => Promise<void>,
): Promise<void> {
  const generation = (cacheMutationGenerations.get(threadId) || 0) + 1;
  cacheMutationGenerations.set(threadId, generation);
  const previous = cacheMutationQueues.get(threadId) || Promise.resolve();
  const next = previous
    .catch(() => undefined)
    .then(async () => {
      if (cacheMutationGenerations.get(threadId) !== generation) {
        return;
      }
      await operation();
  });
  let queued: Promise<void>;
  queued = next
    .catch(() => undefined)
    .finally(() => {
      if (cacheMutationQueues.get(threadId) === queued) {
        cacheMutationQueues.delete(threadId);
      }
    });
  cacheMutationQueues.set(threadId, queued);
  return next;
}

function validTranscript(value: unknown): value is ThreadTranscript {
  if (!value || typeof value !== "object") {
    return false;
  }
  const record = value as Partial<ThreadTranscript>;
  return (
    typeof record.threadId === "string" &&
    Array.isArray(record.messages) &&
    Array.isArray(record.pendingInputs)
  );
}

export async function loadThreadTranscriptCache(
  threadId: string,
): Promise<ThreadTranscript | null> {
  const normalizedThreadId = threadId.trim();
  if (!normalizedThreadId) {
    return null;
  }
  try {
    await waitForCacheMutations(normalizedThreadId);
    const raw = await readFile(transcriptCachePath(normalizedThreadId), "utf8");
    const parsed = JSON.parse(raw) as Partial<CachedThreadTranscriptFile>;
    if (
      parsed.version !== CACHE_VERSION ||
      !validTranscript(parsed.transcript) ||
      parsed.transcript.threadId !== normalizedThreadId
    ) {
      return null;
    }
    return parsed.transcript;
  } catch {
    return null;
  }
}

export async function saveThreadTranscriptCache(
  transcript: ThreadTranscript,
): Promise<void> {
  const threadId = transcript.threadId.trim();
  if (!threadId) {
    return;
  }
  await enqueueLatestCacheMutation(threadId, async () => {
    await mkdir(transcriptCacheDir(), { recursive: true });
    const target = transcriptCachePath(threadId);
    const temp = `${target}.tmp-${process.pid}-${Date.now()}`;
    const payload: CachedThreadTranscriptFile = {
      version: CACHE_VERSION,
      savedAt: new Date().toISOString(),
      transcript,
    };
    await writeFile(temp, JSON.stringify(payload), "utf8");
    await rename(temp, target);
  });
}

export async function clearThreadTranscriptCache(threadId: string): Promise<void> {
  const normalizedThreadId = threadId.trim();
  if (!normalizedThreadId) {
    return;
  }
  await enqueueLatestCacheMutation(normalizedThreadId, async () => {
    await rm(transcriptCachePath(normalizedThreadId), { force: true });
  });
}
