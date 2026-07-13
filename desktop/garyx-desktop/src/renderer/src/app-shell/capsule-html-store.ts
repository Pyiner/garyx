import type { DesktopCapsuleHtmlResult } from '@shared/contracts';

/**
 * App-level shared cache for Capsule served HTML, keyed by `id:revision` so the
 * gallery, the focused preview, and chat capsule cards all reuse one fetch and
 * one cached document. Lives as a module singleton (not AppShell `useState`) so
 * an HTML load for one capsule never re-renders the whole shell or other cards:
 * components subscribe per key through `useSyncExternalStore`, and unchanged
 * keys keep a stable snapshot reference so React bails out of their re-render.
 *
 * The imperative store has no React dependency, so its concurrency, dedupe, and
 * delete-race semantics are covered by headless tests.
 */

export type CapsuleHtmlState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ready'; html: string }
  | { status: 'deleted' }
  | { status: 'error'; message: string };

export function capsuleHtmlCacheKey(id: string, revision: number): string {
  return `${id}:${revision}`;
}

// Stable shared snapshots for the data-less states so `getState` can return a
// referentially-stable value (required by useSyncExternalStore).
const IDLE: CapsuleHtmlState = { status: 'idle' };
const LOADING: CapsuleHtmlState = { status: 'loading' };
const DELETED: CapsuleHtmlState = { status: 'deleted' };

const MAX_CONCURRENT = 4;
export const CAPSULE_HTML_CACHE_MAX_ENTRIES = 32;

type Job = { id: string; revision: number; key: string; gen: number };

type Fetcher = (capsuleId: string) => Promise<DesktopCapsuleHtmlResult>;

function defaultFetcher(capsuleId: string): Promise<DesktopCapsuleHtmlResult> {
  return window.garyxDesktop.getCapsuleHtml(capsuleId);
}

class CapsuleHtmlStore {
  private entries = new Map<string, CapsuleHtmlState>();
  private generationById = new Map<string, number>();
  private inflightCountById = new Map<string, number>();
  private inflightCountByKey = new Map<string, number>();
  private queuedKeys = new Set<string>();
  private queue: Job[] = [];
  private activeCount = 0;
  private listeners = new Set<() => void>();
  private fetcher: Fetcher = defaultFetcher;
  // Cross-store tombstone: a `/serve` 404 here means the capsule is gone, so the
  // rendered-thumbnail store must drop its cached PNGs too. Injected (not a
  // direct import) so the two stores stay decoupled with no import cycle; wired
  // by `capsule-cache.ts`.
  private crossInvalidate: ((id: string) => void) | null = null;

  setCrossInvalidate(fn: ((id: string) => void) | null): void {
    this.crossInvalidate = fn;
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  getState = (key: string): CapsuleHtmlState => this.entries.get(key) ?? IDLE;

  private generationFor(id: string): number {
    return this.generationById.get(id) ?? 0;
  }

  private setEntry(key: string, state: CapsuleHtmlState): void {
    this.entries.delete(key);
    this.entries.set(key, state);
    this.pruneToLimit();
    this.notify();
  }

  private touchEntry(key: string, state: CapsuleHtmlState): void {
    this.entries.delete(key);
    this.entries.set(key, state);
  }

  private pruneToLimit(): boolean {
    let changed = false;
    while (this.entries.size > CAPSULE_HTML_CACHE_MAX_ENTRIES) {
      let evicted = false;
      for (const [key, state] of this.entries) {
        if (
          state.status === 'loading' ||
          this.inflightCountByKey.has(key) ||
          this.queuedKeys.has(key)
        ) {
          continue;
        }
        this.entries.delete(key);
        changed = true;
        evicted = true;
        break;
      }
      if (!evicted) {
        break;
      }
    }
    return changed;
  }

  private notify(): void {
    for (const listener of this.listeners) {
      listener();
    }
  }

  /**
   * Ensure HTML for `id:revision` is loading or loaded. Cached terminal states
   * (ready/deleted) and in-flight loads are no-ops unless `force` is set, which
   * bumps the per-id generation (discarding any in-flight result) and refetches.
   */
  request(id: string, revision: number, options: { force?: boolean } = {}): void {
    const key = capsuleHtmlCacheKey(id, revision);
    const current = this.entries.get(key);
    if (!options.force) {
      if (current && (current.status === 'ready' || current.status === 'deleted')) {
        this.touchEntry(key, current);
        return;
      }
      if (this.inflightCountByKey.has(key) || this.queuedKeys.has(key)) {
        return;
      }
    } else {
      // Force refresh: invalidate any in-flight result for this id and drop a
      // stale queued job so we re-enqueue a fresh fetch.
      this.generationById.set(id, this.generationFor(id) + 1);
      if (this.queuedKeys.has(key)) {
        this.queue = this.queue.filter((job) => job.key !== key);
        this.queuedKeys.delete(key);
      }
    }
    this.setEntry(key, LOADING);
    this.queue.push({ id, revision, key, gen: this.generationFor(id) });
    this.queuedKeys.add(key);
    this.drain();
  }

  /**
   * Called after a Capsule is deleted: bump the id generation so any in-flight
   * fetch is discarded on completion, drop its queued jobs, and tombstone every
   * cached revision so mounted previews/cards flip to deleted immediately.
   */
  invalidateCapsule(id: string): void {
    this.generationById.set(id, this.generationFor(id) + 1);
    this.queue = this.queue.filter((job) => {
      if (job.id === id) {
        this.queuedKeys.delete(job.key);
        return false;
      }
      return true;
    });
    const prefix = `${id}:`;
    let changed = false;
    for (const key of [...this.entries.keys()]) {
      if (key.startsWith(prefix)) {
        this.entries.delete(key);
        this.entries.set(key, DELETED);
        changed = true;
      }
    }
    changed = this.pruneToLimit() || changed;
    if (changed) {
      this.notify();
    }
    this.forgetGenerationIfIdle(id);
  }

  private drain(): void {
    while (this.activeCount < MAX_CONCURRENT && this.queue.length > 0) {
      const job = this.queue.shift()!;
      this.queuedKeys.delete(job.key);
      this.incrementInflight(this.inflightCountByKey, job.key);
      this.incrementInflight(this.inflightCountById, job.id);
      this.activeCount += 1;
      this.run(job);
    }
  }

  private run(job: Job): void {
    this.fetcher(job.id)
      .then((result) => this.settle(job, result, null))
      .catch((error) => this.settle(job, null, error));
  }

  private settle(
    job: Job,
    result: DesktopCapsuleHtmlResult | null,
    error: unknown,
  ): void {
    // Always release the slot and keep draining, even when the result is stale.
    this.activeCount -= 1;
    this.decrementInflight(this.inflightCountByKey, job.key);
    this.decrementInflight(this.inflightCountById, job.id);
    const stale = this.generationFor(job.id) !== job.gen;
    if (!stale) {
      if (error) {
        // Transient/5xx/offline: keep retryable, never mislabel as deleted.
        const message = error instanceof Error ? error.message : String(error);
        this.setEntry(job.key, { status: 'error', message });
      } else if (result && result.status === 'ok') {
        this.setEntry(job.key, { status: 'ready', html: result.html });
      } else {
        // The capsule is gone (`/serve` 404). Tombstone this preview and
        // cross-invalidate the rendered-thumbnail store so the gallery/chat
        // cards for the same id do not keep serving a stale cached PNG.
        this.setEntry(job.key, DELETED);
        this.crossInvalidate?.(job.id);
      }
    } else {
      const releasedLoading = this.releaseOrphanedLoadingEntry(job.key);
      const pruned = this.pruneToLimit();
      if (releasedLoading || pruned) {
        this.notify();
      }
    }
    this.drain();
    this.forgetGenerationIfIdle(job.id);
  }

  private releaseOrphanedLoadingEntry(key: string): boolean {
    if (
      this.entries.get(key) !== LOADING ||
      this.inflightCountByKey.has(key) ||
      this.queuedKeys.has(key)
    ) {
      return false;
    }
    this.entries.delete(key);
    return true;
  }

  private incrementInflight(counts: Map<string, number>, key: string): void {
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }

  private decrementInflight(counts: Map<string, number>, key: string): void {
    const next = (counts.get(key) ?? 1) - 1;
    if (next > 0) {
      counts.set(key, next);
    } else {
      counts.delete(key);
    }
  }

  private forgetGenerationIfIdle(id: string): void {
    if (
      !this.inflightCountById.has(id) &&
      !this.queue.some((job) => job.id === id)
    ) {
      this.generationById.delete(id);
    }
  }

  // --- test-only seams ------------------------------------------------------
  __setFetcherForTest(fetcher: Fetcher): void {
    this.fetcher = fetcher;
  }

  __reset(): void {
    this.entries.clear();
    this.generationById.clear();
    this.inflightCountById.clear();
    this.inflightCountByKey.clear();
    this.queuedKeys.clear();
    this.queue = [];
    this.activeCount = 0;
    this.listeners.clear();
    this.fetcher = defaultFetcher;
    this.crossInvalidate = null;
  }

  __activeCount(): number {
    return this.activeCount;
  }

  __entryCount(): number {
    return this.entries.size;
  }

  __generationCount(): number {
    return this.generationById.size;
  }
}

export const capsuleHtmlStore = new CapsuleHtmlStore();

export function __setCapsuleHtmlFetcherForTest(
  fetcher: (capsuleId: string) => Promise<DesktopCapsuleHtmlResult>,
): void {
  capsuleHtmlStore.__setFetcherForTest(fetcher);
}

export function __resetCapsuleHtmlStoreForTest(): void {
  capsuleHtmlStore.__reset();
}
