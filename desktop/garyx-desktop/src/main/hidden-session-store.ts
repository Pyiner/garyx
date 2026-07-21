import { randomUUID } from 'node:crypto';
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';

import { app } from 'electron';

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
 * - Serialized mutations — adds/removes chain on one promise, so concurrent
 *   side-chat creations merge instead of overwriting each other.
 * - Gateway-scope partitions — every entry lives under its creating
 *   gateway's normalized URL. A late create or lifecycle result from a
 *   previous gateway mutates only that gateway's partition; equal thread
 *   ids across gateways cannot collide.
 */

const HIDDEN_SESSIONS_FILE_NAME = 'garyx-hidden-sessions.json';

type HiddenSessionPartitions = Record<
  string,
  Record<string, DesktopThreadSummary>
>;

let cachedPartitions: HiddenSessionPartitions | null = null;
let mutationChain: Promise<void> = Promise.resolve();

function hiddenSessionsFilePath(): string {
  return join(app.getPath('userData'), HIDDEN_SESSIONS_FILE_NAME);
}

function normalizedScope(scope: string | null | undefined): string {
  return (scope || '').trim();
}

async function loadPartitions(): Promise<HiddenSessionPartitions> {
  if (cachedPartitions) {
    return cachedPartitions;
  }
  try {
    const raw = await readFile(hiddenSessionsFilePath(), 'utf8');
    const parsed = JSON.parse(raw) as HiddenSessionPartitions;
    cachedPartitions =
      parsed && typeof parsed === 'object' && !Array.isArray(parsed)
        ? parsed
        : {};
  } catch {
    cachedPartitions = {};
  }
  return cachedPartitions;
}

async function persistPartitions(
  partitions: HiddenSessionPartitions,
): Promise<void> {
  const filePath = hiddenSessionsFilePath();
  await mkdir(dirname(filePath), { recursive: true });
  const tempPath = `${filePath}.tmp-${process.pid}-${Date.now().toString(36)}-${randomUUID().slice(0, 8)}`;
  await writeFile(tempPath, JSON.stringify(partitions, null, 2), 'utf8');
  await rename(tempPath, filePath);
}

function enqueue(mutate: (partitions: HiddenSessionPartitions) => boolean): Promise<void> {
  const run = mutationChain.then(async () => {
    const partitions = await loadPartitions();
    if (mutate(partitions)) {
      await persistPartitions(partitions);
    }
  });
  mutationChain = run.catch(() => {});
  return run;
}

/** Load the store into memory (idempotent); hydration awaits this once. */
export async function ensureHiddenSessionsLoaded(): Promise<void> {
  await loadPartitions();
}

/** The retained hidden summaries for one gateway scope (memory snapshot). */
export function listHiddenSessions(
  scope: string | null | undefined,
): DesktopThreadSummary[] {
  const partition = cachedPartitions?.[normalizedScope(scope)];
  return partition ? Object.values(partition) : [];
}

export function rememberHiddenSession(
  scope: string | null | undefined,
  thread: DesktopThreadSummary,
): Promise<void> {
  const key = normalizedScope(scope);
  if (!key || !thread.id) {
    return Promise.resolve();
  }
  return enqueue((partitions) => {
    const partition = (partitions[key] ||= {});
    partition[thread.id] = thread;
    return true;
  });
}

export function forgetHiddenSession(
  scope: string | null | undefined,
  threadId: string,
): Promise<void> {
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
}
