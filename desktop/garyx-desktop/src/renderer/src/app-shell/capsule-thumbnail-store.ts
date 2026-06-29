import type { DesktopCapsuleThumbnailResult } from '@shared/contracts';

/**
 * App-level shared cache for *rendered* Capsule thumbnail images, mirroring
 * `capsule-html-store.ts` but rendition-aware. The gallery card (16:10) and
 * chat card (16:9) crop the same capsule differently, so the cache key is
 * `id:revision:rendition` — a bare `id:revision` would let a 16:10 image be
 * served into a 16:9 card. Lives as a module singleton (not AppShell state) so
 * one card's image load never re-renders the whole shell or other cards:
 * components subscribe per key through `useSyncExternalStore`, and unchanged
 * keys keep a stable snapshot reference so React bails out of their re-render.
 *
 * The main process renders the (untrusted) HTML once into a cached PNG and
 * returns a `data:` URL, so the renderer never mounts a live iframe for a card.
 * The imperative store has no React dependency, so its concurrency, dedupe, and
 * delete-race semantics are covered by headless tests.
 */

export type CapsuleThumbnailState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ready'; dataUrl: string }
  | { status: 'deleted' }
  | { status: 'error'; message: string };

export interface CapsuleThumbnailRendition {
  aspectWidth: number;
  aspectHeight: number;
}

/** Gallery card preview (16:10). */
export const GALLERY_RENDITION: CapsuleThumbnailRendition = {
  aspectWidth: 16,
  aspectHeight: 10,
};
/** Chat transcript capsule card (16:9). */
export const CHAT_CARD_RENDITION: CapsuleThumbnailRendition = {
  aspectWidth: 16,
  aspectHeight: 9,
};

function renditionToken(rendition: CapsuleThumbnailRendition): string {
  return `${rendition.aspectWidth}x${rendition.aspectHeight}`;
}

export function capsuleThumbnailCacheKey(
  id: string,
  revision: number,
  rendition: CapsuleThumbnailRendition,
): string {
  return `${id}:${revision}:${renditionToken(rendition)}`;
}

// Stable shared snapshots for the data-less states so `getState` can return a
// referentially-stable value (required by useSyncExternalStore).
const IDLE: CapsuleThumbnailState = { status: 'idle' };
const LOADING: CapsuleThumbnailState = { status: 'loading' };
const DELETED: CapsuleThumbnailState = { status: 'deleted' };

const MAX_CONCURRENT = 4;

type Job = {
  id: string;
  revision: number;
  rendition: CapsuleThumbnailRendition;
  key: string;
  gen: number;
};

type Fetcher = (
  capsuleId: string,
  revision: number,
  rendition: CapsuleThumbnailRendition,
) => Promise<DesktopCapsuleThumbnailResult>;

function defaultFetcher(
  capsuleId: string,
  revision: number,
  rendition: CapsuleThumbnailRendition,
): Promise<DesktopCapsuleThumbnailResult> {
  return window.garyxDesktop.getCapsuleThumbnail(capsuleId, revision, rendition);
}

class CapsuleThumbnailStore {
  private entries = new Map<string, CapsuleThumbnailState>();
  private generationById = new Map<string, number>();
  private inflightKeys = new Set<string>();
  private queuedKeys = new Set<string>();
  private queue: Job[] = [];
  private activeCount = 0;
  private listeners = new Set<() => void>();
  private fetcher: Fetcher = defaultFetcher;
  // Cross-store tombstone: a `/serve` 404 discovered while rendering a thumbnail
  // means the capsule is gone, so the HTML store (focused preview) must drop its
  // cached document too. Injected (no import cycle); wired by `capsule-cache.ts`.
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

  getState = (key: string): CapsuleThumbnailState => this.entries.get(key) ?? IDLE;

  private generationFor(id: string): number {
    return this.generationById.get(id) ?? 0;
  }

  private setEntry(key: string, state: CapsuleThumbnailState): void {
    this.entries.set(key, state);
    this.notify();
  }

  private notify(): void {
    for (const listener of this.listeners) {
      listener();
    }
  }

  /**
   * Ensure the thumbnail for `id:revision:rendition` is loading or loaded.
   * Cached terminal states (ready/deleted) and in-flight loads are no-ops
   * unless `force` is set, which bumps the per-id generation (discarding any
   * in-flight result) and re-renders.
   */
  request(
    id: string,
    revision: number,
    rendition: CapsuleThumbnailRendition,
    options: { force?: boolean } = {},
  ): void {
    const key = capsuleThumbnailCacheKey(id, revision, rendition);
    const current = this.entries.get(key);
    if (!options.force) {
      if (current && (current.status === 'ready' || current.status === 'deleted')) {
        return;
      }
      if (this.inflightKeys.has(key) || this.queuedKeys.has(key)) {
        return;
      }
    } else {
      // Force refresh: invalidate any in-flight result for this id and drop a
      // stale queued job so we re-enqueue a fresh render.
      this.generationById.set(id, this.generationFor(id) + 1);
      if (this.queuedKeys.has(key)) {
        this.queue = this.queue.filter((job) => job.key !== key);
        this.queuedKeys.delete(key);
      }
    }
    this.setEntry(key, LOADING);
    this.queue.push({ id, revision, rendition, key, gen: this.generationFor(id) });
    this.queuedKeys.add(key);
    this.drain();
  }

  /**
   * Called after a Capsule is deleted: bump the id generation so any in-flight
   * render is discarded on completion, drop its queued jobs, and tombstone
   * every cached rendition/revision so mounted cards flip to deleted immediately.
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
    for (const key of this.entries.keys()) {
      if (key.startsWith(prefix)) {
        this.entries.set(key, DELETED);
        changed = true;
      }
    }
    if (changed) {
      this.notify();
    }
  }

  private drain(): void {
    while (this.activeCount < MAX_CONCURRENT && this.queue.length > 0) {
      const job = this.queue.shift()!;
      this.queuedKeys.delete(job.key);
      this.inflightKeys.add(job.key);
      this.activeCount += 1;
      this.run(job);
    }
  }

  private run(job: Job): void {
    this.fetcher(job.id, job.revision, job.rendition)
      .then((result) => this.settle(job, result, null))
      .catch((error) => this.settle(job, null, error));
  }

  private settle(
    job: Job,
    result: DesktopCapsuleThumbnailResult | null,
    error: unknown,
  ): void {
    // Always release the slot and keep draining, even when the result is stale.
    this.activeCount -= 1;
    this.inflightKeys.delete(job.key);
    const stale = this.generationFor(job.id) !== job.gen;
    if (!stale) {
      if (error) {
        const message = error instanceof Error ? error.message : String(error);
        this.setEntry(job.key, { status: 'error', message });
      } else if (result && result.status === 'ok') {
        this.setEntry(job.key, { status: 'ready', dataUrl: result.dataUrl });
      } else if (result && result.status === 'error') {
        // Transient render/network failure: keep retryable, never tombstone.
        this.setEntry(job.key, { status: 'error', message: result.message });
      } else if (result && result.status === 'deleted') {
        // The whole capsule is gone (a `/serve` 404). Tombstone *every*
        // rendition/revision for this id — not just the requested key — so a
        // sibling card at another rendition (gallery 16:10 vs chat 16:9) does
        // not keep serving a stale `ready` image. Mirrors the iOS
        // `evictingCapsule` (all `(id, *, *)`) semantics, and cross-invalidates
        // the HTML store so a re-opened focused preview is not stale either.
        this.invalidateCapsule(job.id);
        this.crossInvalidate?.(job.id);
      } else {
        // Unexpected empty result: tombstone just this key (retryable shape unknown).
        this.setEntry(job.key, DELETED);
      }
    }
    this.drain();
  }

  // --- test-only seams ------------------------------------------------------
  __setFetcherForTest(fetcher: Fetcher): void {
    this.fetcher = fetcher;
  }

  __reset(): void {
    this.entries.clear();
    this.generationById.clear();
    this.inflightKeys.clear();
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
}

export const capsuleThumbnailStore = new CapsuleThumbnailStore();

export function __setCapsuleThumbnailFetcherForTest(fetcher: Fetcher): void {
  capsuleThumbnailStore.__setFetcherForTest(fetcher);
}

export function __resetCapsuleThumbnailStoreForTest(): void {
  capsuleThumbnailStore.__reset();
}
