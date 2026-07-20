import type { DesktopWorkspaceGitStatus } from "@shared/contracts";

export const WORKSPACE_GIT_STATUS_CACHE_MAX_ENTRIES = 64;
export const WORKSPACE_GIT_STATUS_CACHE_TTL_MS = 30_000;

interface WorkspaceGitStatusCacheEntry {
  expiresAt: number;
  status: DesktopWorkspaceGitStatus;
}

export class WorkspaceGitStatusCache {
  private readonly maxEntries: number;
  private readonly ttlMs: number;
  private entries = new Map<string, WorkspaceGitStatusCacheEntry>();
  private generation = 0;

  constructor(options: { maxEntries?: number; ttlMs?: number } = {}) {
    this.maxEntries =
      options.maxEntries ?? WORKSPACE_GIT_STATUS_CACHE_MAX_ENTRIES;
    this.ttlMs = options.ttlMs ?? WORKSPACE_GIT_STATUS_CACHE_TTL_MS;
  }

  /** Gateway switch: cached statuses belong to the previous machine, and a
   *  late in-flight load from before the switch must not repopulate the
   *  cache — `set` drops writes whose generation predates the clear. */
  clear(): void {
    this.entries.clear();
    this.generation += 1;
  }

  currentGeneration(): number {
    return this.generation;
  }

  get(workspacePath: string, now = Date.now()): DesktopWorkspaceGitStatus | null {
    const key = workspacePath.trim();
    const entry = key ? this.entries.get(key) : undefined;
    if (!entry) {
      return null;
    }
    if (entry.expiresAt <= now) {
      this.entries.delete(key);
      return null;
    }
    this.entries.delete(key);
    this.entries.set(key, entry);
    return entry.status;
  }

  set(
    workspacePath: string,
    status: DesktopWorkspaceGitStatus,
    now = Date.now(),
    generation = this.generation,
  ): void {
    const key = workspacePath.trim();
    if (!key || generation !== this.generation) {
      return;
    }
    this.entries.delete(key);
    this.entries.set(key, {
      expiresAt: now + this.ttlMs,
      status,
    });
    while (this.entries.size > this.maxEntries) {
      const oldestKey = this.entries.keys().next().value;
      if (typeof oldestKey !== "string") {
        break;
      }
      this.entries.delete(oldestKey);
    }
  }

  invalidateNegative(workspacePath: string): boolean {
    const key = workspacePath.trim();
    const entry = key ? this.entries.get(key) : undefined;
    if (!entry || entry.status.isGitRepo) {
      return false;
    }
    this.entries.delete(key);
    return true;
  }
}

export async function loadWorkspaceGitStatusCached(options: {
  cache: WorkspaceGitStatusCache;
  load: () => Promise<DesktopWorkspaceGitStatus>;
  now?: number;
  workspacePath: string;
}): Promise<DesktopWorkspaceGitStatus> {
  const cached = options.cache.get(options.workspacePath, options.now);
  if (cached) {
    return cached;
  }
  const generation = options.cache.currentGeneration();
  const status = await options.load();
  options.cache.set(options.workspacePath, status, options.now, generation);
  return status;
}

export const workspaceGitStatusCache = new WorkspaceGitStatusCache();
