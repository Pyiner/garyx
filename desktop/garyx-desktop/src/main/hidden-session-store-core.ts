import { randomUUID } from 'node:crypto';
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { dirname } from 'node:path';

import type { DesktopThreadSummary } from '@shared/contracts';

/**
 * Durable store for hidden session summaries (side-chat children).
 *
 * These threads never appear in the gateway's regular thread list, so their
 * summaries need an owner that survives full-state refreshes, restarts, and
 * gateway switches. This store IS that single owner:
 *
 * - Its own file — the main desktop-state file has many independent writers
 *   whose read-modify-write cycles would race a shared slice; nothing else
 *   writes this domain.
 * - Single-flight initialization + serialized mutations — every load shares
 *   one in-flight read (a late duplicate read can never overwrite a fresher
 *   cache), and adds/removes chain on one promise so concurrent side-chat
 *   creations merge instead of overwriting each other.
 * - Gateway-scope partitions — every entry lives under its creating
 *   gateway's normalized URL. A late create or lifecycle result from a
 *   previous gateway mutates only that gateway's partition; equal thread
 *   ids across gateways cannot collide.
 *
 * The factory takes the storage path lazily and accepts injectable IO so
 * tests can gate reads/writes and exercise the real interleavings (paused
 * load with a mutation queued behind it, overlapping persists) instead of
 * only the happy path.
 */

type HiddenSessionPartitions = Record<
  string,
  Record<string, DesktopThreadSummary>
>;

export interface HiddenSessionStoreIo {
  readFile(filePath: string): Promise<string>;
  writeFile(filePath: string, data: string): Promise<void>;
  rename(fromPath: string, toPath: string): Promise<void>;
  mkdir(dirPath: string): Promise<void>;
}

const defaultIo: HiddenSessionStoreIo = {
  readFile: (filePath) => readFile(filePath, 'utf8'),
  writeFile: (filePath, data) => writeFile(filePath, data, 'utf8'),
  rename: (fromPath, toPath) => rename(fromPath, toPath),
  mkdir: async (dirPath) => {
    await mkdir(dirPath, { recursive: true });
  },
};

export interface HiddenSessionStore {
  /** Load the store into memory (single-flight, idempotent). */
  ensureLoaded(): Promise<void>;
  /** The retained hidden summaries for one gateway scope (memory snapshot). */
  list(scope: string | null | undefined): DesktopThreadSummary[];
  remember(
    scope: string | null | undefined,
    thread: DesktopThreadSummary,
  ): Promise<void>;
  forget(
    scope: string | null | undefined,
    threadId: string,
  ): Promise<void>;
}

function normalizedScope(scope: string | null | undefined): string {
  return (scope || '').trim();
}

export function createHiddenSessionStore(
  filePathProvider: () => string,
  io: HiddenSessionStoreIo = defaultIo,
): HiddenSessionStore {
  let cachedPartitions: HiddenSessionPartitions | null = null;
  let loadPromise: Promise<HiddenSessionPartitions> | null = null;
  let mutationChain: Promise<void> = Promise.resolve();

  async function readPartitionsOnce(): Promise<HiddenSessionPartitions> {
    try {
      const raw = await io.readFile(filePathProvider());
      const parsed = JSON.parse(raw) as HiddenSessionPartitions;
      return parsed && typeof parsed === 'object' && !Array.isArray(parsed)
        ? parsed
        : {};
    } catch {
      return {};
    }
  }

  function loadPartitions(): Promise<HiddenSessionPartitions> {
    if (cachedPartitions) {
      return Promise.resolve(cachedPartitions);
    }
    // Single-flight: every concurrent caller shares one read, and the cache
    // is only ever installed once — a late duplicate read cannot overwrite
    // a cache that mutations have already advanced.
    if (!loadPromise) {
      loadPromise = readPartitionsOnce().then((partitions) => {
        cachedPartitions ??= partitions;
        return cachedPartitions;
      });
    }
    return loadPromise;
  }

  async function persistPartitions(
    partitions: HiddenSessionPartitions,
  ): Promise<void> {
    const filePath = filePathProvider();
    await io.mkdir(dirname(filePath));
    const tempPath = `${filePath}.tmp-${process.pid}-${Date.now().toString(36)}-${randomUUID().slice(0, 8)}`;
    await io.writeFile(tempPath, JSON.stringify(partitions, null, 2));
    await io.rename(tempPath, filePath);
  }

  function enqueue(
    mutate: (partitions: HiddenSessionPartitions) => boolean,
  ): Promise<void> {
    const run = mutationChain.then(async () => {
      const partitions = await loadPartitions();
      if (mutate(partitions)) {
        await persistPartitions(partitions);
      }
    });
    mutationChain = run.catch(() => {});
    return run;
  }

  return {
    async ensureLoaded() {
      await loadPartitions();
    },
    list(scope) {
      const partition = cachedPartitions?.[normalizedScope(scope)];
      return partition ? Object.values(partition) : [];
    },
    remember(scope, thread) {
      const key = normalizedScope(scope);
      if (!key || !thread.id) {
        return Promise.resolve();
      }
      return enqueue((partitions) => {
        const partition = (partitions[key] ||= {});
        partition[thread.id] = thread;
        return true;
      });
    },
    forget(scope, threadId) {
      const key = normalizedScope(scope);
      if (!key || !threadId) {
        return Promise.resolve();
      }
      return enqueue((partitions) => {
        const partition = partitions[key];
        if (!partition || !(threadId in partition)) {
          return false;
        }
        delete partition[threadId];
        if (Object.keys(partition).length === 0) {
          delete partitions[key];
        }
        return true;
      });
    },
  };
}
