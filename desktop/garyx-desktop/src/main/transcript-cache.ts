import {
  mkdir,
  readFile,
  readdir,
  rename,
  rm,
  stat,
  utimes,
  writeFile,
} from "node:fs/promises";
import { join } from "node:path";

import { app } from "electron";

import type {
  CachedThreadTranscript,
  RenderState,
  ThreadTranscript,
} from "@shared/contracts";

export type { CachedThreadTranscript };

// Not bumped when `renderState` was added: it's an optional field, so existing
// v1 caches still load (with `renderState` undefined → graceful degradation to
// an empty render until the first live frame).
const CACHE_VERSION = 1;
export const MAX_TRANSCRIPT_CACHE_BYTES = 48 * 1024 * 1024;
export const MAX_TRANSCRIPT_CACHE_RECORDS = 240;

interface CachedThreadTranscriptFile {
  version: number;
  savedAt: string;
  transcript: ThreadTranscript;
  // Optional offline render snapshot so a cold/offline thread open can render
  // its folded history without a gateway round-trip.
  renderState?: RenderState | null;
}

export interface TranscriptCacheStoreOptions {
  directory: () => string;
  maxBytes?: number;
  maxRecords?: number;
  now?: () => Date;
}

interface CacheRecord {
  fileName: string;
  byteCount: number;
  lastAccessAt: number;
}

/**
 * Cache identity is the (gatewayScope, threadId) PAIR: thread ids are only
 * unique per gateway, so an unpartitioned cache would flash gateway A's
 * transcript into gateway B's thread of the same id. The scope is encoded
 * into the file name, so entries from different gateways can never collide
 * and a gateway switch needs no cache invalidation at all.
 */
function cacheKey(scope: string, threadId: string): string {
  return `${scope}\n${threadId}`;
}

function cacheFileName(scope: string, threadId: string): string {
  return `${Buffer.from(cacheKey(scope, threadId), "utf8").toString("base64url")}.json`;
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

export class TranscriptCacheStore {
  private readonly directory: () => string;
  private readonly maxBytes: number;
  private readonly maxRecords: number;
  private readonly now: () => Date;
  private operationQueue: Promise<void> = Promise.resolve();
  private mutationGenerationByThread = new Map<string, number>();

  constructor(options: TranscriptCacheStoreOptions) {
    this.directory = options.directory;
    this.maxBytes = options.maxBytes ?? MAX_TRANSCRIPT_CACHE_BYTES;
    this.maxRecords = options.maxRecords ?? MAX_TRANSCRIPT_CACHE_RECORDS;
    this.now = options.now ?? (() => new Date());
  }

  async load(
    scope: string,
    threadId: string,
  ): Promise<CachedThreadTranscript | null> {
    const normalizedThreadId = threadId.trim();
    if (!normalizedThreadId) {
      return null;
    }
    return this.serialize(async () => {
      const target = this.pathForThread(scope, normalizedThreadId);
      try {
        const raw = await readFile(target, "utf8");
        const parsed = JSON.parse(raw) as Partial<CachedThreadTranscriptFile>;
        if (
          parsed.version !== CACHE_VERSION ||
          !validTranscript(parsed.transcript) ||
          parsed.transcript.threadId !== normalizedThreadId
        ) {
          return null;
        }
        const accessedAt = this.now();
        await utimes(target, accessedAt, accessedAt).catch(() => undefined);
        return {
          transcript: parsed.transcript,
          renderState: parsed.renderState ?? null,
        };
      } catch {
        return null;
      }
    });
  }

  save(
    scope: string,
    transcript: ThreadTranscript,
    renderState?: RenderState | null,
  ): Promise<void> {
    const threadId = transcript.threadId.trim();
    if (!threadId) {
      return Promise.resolve();
    }
    return this.enqueueLatestMutation(cacheKey(scope, threadId), async () => {
      const directory = this.directory();
      await mkdir(directory, { recursive: true });
      const target = this.pathForThread(scope, threadId);
      const savedAt = this.now();
      const temp = `${target}.tmp-${process.pid}-${savedAt.getTime()}`;
      const payload: CachedThreadTranscriptFile = {
        version: CACHE_VERSION,
        savedAt: savedAt.toISOString(),
        transcript,
        renderState: renderState ?? null,
      };
      try {
        await writeFile(temp, JSON.stringify(payload), "utf8");
        await rename(temp, target);
        await utimes(target, savedAt, savedAt).catch(() => undefined);
      } finally {
        await rm(temp, { force: true }).catch(() => undefined);
      }
      await this.pruneToLimits();
    });
  }

  clear(scope: string, threadId: string): Promise<void> {
    const normalizedThreadId = threadId.trim();
    if (!normalizedThreadId) {
      return Promise.resolve();
    }
    return this.enqueueLatestMutation(
      cacheKey(scope, normalizedThreadId),
      async () => {
        await rm(this.pathForThread(scope, normalizedThreadId), {
          force: true,
        });
      },
    );
  }

  prune(): Promise<void> {
    return this.serialize(() => this.pruneToLimits());
  }

  private pathForThread(scope: string, threadId: string): string {
    return join(this.directory(), cacheFileName(scope, threadId));
  }

  private enqueueLatestMutation(
    threadId: string,
    operation: () => Promise<void>,
  ): Promise<void> {
    const generation =
      (this.mutationGenerationByThread.get(threadId) ?? 0) + 1;
    this.mutationGenerationByThread.set(threadId, generation);
    return this.serialize(async () => {
      if (this.mutationGenerationByThread.get(threadId) !== generation) {
        return;
      }
      try {
        await operation();
      } finally {
        if (this.mutationGenerationByThread.get(threadId) === generation) {
          this.mutationGenerationByThread.delete(threadId);
        }
      }
    });
  }

  private serialize<T>(operation: () => Promise<T>): Promise<T> {
    const result = this.operationQueue.catch(() => undefined).then(operation);
    this.operationQueue = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }

  private async pruneToLimits(): Promise<void> {
    let names: string[];
    try {
      names = (await readdir(this.directory())).filter((name) =>
        name.endsWith(".json"),
      );
    } catch {
      return;
    }

    const records = (
      await Promise.all(
        names.map(async (fileName): Promise<CacheRecord | null> => {
          try {
            const metadata = await stat(join(this.directory(), fileName));
            return metadata.isFile()
              ? {
                  fileName,
                  byteCount: metadata.size,
                  lastAccessAt: metadata.mtimeMs,
                }
              : null;
          } catch {
            return null;
          }
        }),
      )
    ).filter((record): record is CacheRecord => record !== null);

    let recordCount = records.length;
    let totalBytes = records.reduce(
      (total, record) => total + record.byteCount,
      0,
    );
    if (recordCount <= this.maxRecords && totalBytes <= this.maxBytes) {
      return;
    }

    records.sort(
      (left, right) =>
        left.lastAccessAt - right.lastAccessAt ||
        left.fileName.localeCompare(right.fileName),
    );
    for (const record of records) {
      if (recordCount <= this.maxRecords && totalBytes <= this.maxBytes) {
        break;
      }
      try {
        await rm(join(this.directory(), record.fileName), { force: true });
        recordCount -= 1;
        totalBytes -= record.byteCount;
      } catch {
        // Best-effort cache: keep trying newer candidates if one file is locked.
      }
    }
  }
}

const transcriptCache = new TranscriptCacheStore({
  directory: () => join(app.getPath("userData"), "transcript-cache"),
});

export function loadThreadTranscriptCache(
  scope: string,
  threadId: string,
): Promise<CachedThreadTranscript | null> {
  return transcriptCache.load(scope, threadId);
}

export function saveThreadTranscriptCache(
  scope: string,
  transcript: ThreadTranscript,
  renderState?: RenderState | null,
): Promise<void> {
  return transcriptCache.save(scope, transcript, renderState);
}

export function clearThreadTranscriptCache(
  scope: string,
  threadId: string,
): Promise<void> {
  return transcriptCache.clear(scope, threadId);
}

export function pruneThreadTranscriptCache(): Promise<void> {
  return transcriptCache.prune();
}
