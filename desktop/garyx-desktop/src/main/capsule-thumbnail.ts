import { createHash } from "node:crypto";
import { promises as fs } from "node:fs";
import { join } from "node:path";

import { app, BrowserWindow } from "electron";

import type {
  DesktopCapsuleThumbnailResult,
  DesktopSettings,
} from "@shared/contracts";

import { getCapsuleHtml } from "./gary-client";

/**
 * Renders a Capsule's served HTML once into a fixed-aspect thumbnail PNG and
 * caches it on disk, keyed by `(id, revision, rendition)`. The gallery and chat
 * cards show this cached image (zero live iframe); the focused preview keeps a
 * live, interactive iframe. This is the desktop equivalent of the iOS
 * `GaryxCapsuleThumbnailRenderer` + `GaryxCapsuleThumbnailDiskStore` pair.
 *
 * Why a rendition is part of the key (not a bare `id:revision`): the gallery is
 * 16:10 and the chat card is 16:9, so the same capsule is cropped differently.
 * A bare key would serve a 16:10 image into a 16:9 card.
 *
 * Security: capsule HTML is untrusted. It is rendered in a hidden, sandboxed
 * `BrowserWindow` with no preload, no Node integration, context isolation on,
 * and a throwaway in-memory session — never given access to the app surface.
 */

type CapsuleThumbnailRendition = {
  aspectWidth: number;
  aspectHeight: number;
};

// Layout viewport the capsule HTML is rendered into (CSS px), matching the
// long-standing thumbnail virtual canvas the desktop card used. A device-width
// viewport is injected (see `ensureMobileViewport`) so a capsule that declares
// none does NOT fall back to the ~980px desktop width and leave side gutters.
const LAYOUT_WIDTH = 1024;
// Output PNG is rendered at 2x the layout band for crisp downscaling into cards.
const PIXEL_SCALE = 2;
// Capsule-dark neutral (~#0a0e16): cropped/letterboxed regions read as
// intentional backing instead of the card's translucent fill. Matches iOS.
const BACKING_COLOR = "#0a0e16";
// Cap concurrent offscreen renders so a fresh, all-miss gallery does not spin
// up many hidden windows at once. Unlike a display planner this never starves a
// card: every visible card still gets its one-shot render as the queue drains.
const MAX_CONCURRENT_RENDERS = 2;
// Settle delay after `did-finish-load` for final layout / inline-JS paint.
const SETTLE_MS = 160;
// Safety net: capsule HTML is self-contained (CSP blocks external fetches) so a
// load is fast; this only guards a pathological page from hanging a slot.
const LOAD_TIMEOUT_MS = 6000;

// LRU caps, mirroring the iOS disk store (48 MB / 240 records).
const MAX_CACHE_BYTES = 48 * 1024 * 1024;
const MAX_CACHE_RECORDS = 240;

const CACHE_SUBDIR = join("GaryxCapsuleThumbnailCache", "v1");

function renditionToken(rendition: CapsuleThumbnailRendition): string {
  const w = Math.max(1, Math.trunc(rendition.aspectWidth));
  const h = Math.max(1, Math.trunc(rendition.aspectHeight));
  return `${w}x${h}`;
}

/** Stable, filesystem-key token, e.g. `<id>.r3.16x10`. */
export function capsuleThumbnailStorageToken(
  id: string,
  revision: number,
  rendition: CapsuleThumbnailRendition,
): string {
  return `${id.trim()}.r${revision}.${renditionToken(rendition)}`;
}

function pngBufferToDataUrl(buffer: Buffer): string {
  return `data:image/png;base64,${buffer.toString("base64")}`;
}

/**
 * Returns `html` guaranteed to carry a device-width viewport meta. Inserts it
 * right after an existing `<head …>` open tag, otherwise prepends it (mirroring
 * the gateway's CSP injection and the iOS `GaryxCapsuleViewport`). HTML that
 * already declares a viewport is returned unchanged.
 */
export function ensureMobileViewport(html: string): string {
  const VIEWPORT_META =
    '<meta name="viewport" content="width=device-width, initial-scale=1">';
  if (/<meta[^>]*name\s*=\s*["']?viewport["']?/i.test(html)) {
    return html;
  }
  const headOpen = /<head\b[^>]*>/i.exec(html);
  if (headOpen) {
    const insertAt = headOpen.index + headOpen[0].length;
    return html.slice(0, insertAt) + VIEWPORT_META + html.slice(insertAt);
  }
  return VIEWPORT_META + html;
}

/** Caps concurrent offscreen renders; releasing hands the slot to the next FIFO waiter. */
class RenderGate {
  private active = 0;
  private waiters: Array<() => void> = [];

  constructor(private readonly limit: number) {}

  async acquire(): Promise<void> {
    if (this.active < this.limit) {
      this.active += 1;
      return;
    }
    await new Promise<void>((resolve) => this.waiters.push(resolve));
  }

  release(): void {
    const next = this.waiters.shift();
    if (next) {
      next();
    } else {
      this.active = Math.max(0, this.active - 1);
    }
  }
}

const renderGate = new RenderGate(MAX_CONCURRENT_RENDERS);

/**
 * Render the capsule HTML in a hidden sandboxed window and capture the top
 * `16:rendition` band (top-anchored cover). Returns PNG bytes, or null on a
 * render failure (transient — the next sighting re-renders).
 */
async function renderThumbnailPng(
  html: string,
  rendition: CapsuleThumbnailRendition,
): Promise<Buffer | null> {
  await renderGate.acquire();
  let window: BrowserWindow | null = null;
  try {
    const layoutHeight = Math.round(
      (LAYOUT_WIDTH * Math.max(1, rendition.aspectHeight)) /
        Math.max(1, rendition.aspectWidth),
    );
    window = new BrowserWindow({
      show: false,
      width: LAYOUT_WIDTH,
      height: layoutHeight,
      useContentSize: true,
      backgroundColor: BACKING_COLOR,
      webPreferences: {
        // Untrusted capsule HTML: lock it down hard.
        sandbox: true,
        nodeIntegration: false,
        contextIsolation: true,
        webSecurity: true,
        javascript: true,
        // No preload, no shared session: a throwaway in-memory partition.
        partition: `capsule-thumbnail-${Date.now()}-${Math.random()
          .toString(36)
          .slice(2)}`,
        backgroundThrottling: false,
        // Paint content at 2x so the captured band downscales crisply.
        zoomFactor: PIXEL_SCALE,
      },
    });

    const finished = new Promise<void>((resolve) => {
      let settled = false;
      const done = () => {
        if (settled) {
          return;
        }
        settled = true;
        resolve();
      };
      window!.webContents.once("did-finish-load", done);
      window!.webContents.once("did-fail-load", done);
      setTimeout(done, LOAD_TIMEOUT_MS);
    });

    const dataUrl =
      "data:text/html;charset=utf-8," +
      encodeURIComponent(ensureMobileViewport(html));
    await window.loadURL(dataUrl);
    await finished;
    // Brief settle for final layout / inline-JS paint before capturing.
    await new Promise<void>((resolve) => setTimeout(resolve, SETTLE_MS));

    if (window.isDestroyed()) {
      return null;
    }
    // Capture the top band only (cover, top-anchored). `zoomFactor` already
    // scales painted content, so the captured NativeImage is ~2x; resize to a
    // deterministic pixel size to absorb DPR differences across displays.
    const image = await window.webContents.capturePage({
      x: 0,
      y: 0,
      width: LAYOUT_WIDTH,
      height: layoutHeight,
    });
    if (image.isEmpty()) {
      return null;
    }
    const pixelWidth = Math.round(LAYOUT_WIDTH * PIXEL_SCALE);
    const pixelHeight = Math.round(layoutHeight * PIXEL_SCALE);
    const resized = image.resize({ width: pixelWidth, height: pixelHeight });
    const png = resized.toPNG();
    return png.length > 0 ? png : null;
  } catch {
    return null;
  } finally {
    if (window && !window.isDestroyed()) {
      window.destroy();
    }
    renderGate.release();
  }
}

/**
 * On-disk PNG cache, keyed by the `(id, revision, rendition)` storage token. A
 * small JSON index (mirroring the iOS `GaryxCapsuleThumbnailDiskStore`) maps
 * tokens to files, which lets a 404 evict every rendition/revision of one
 * capsule by id without scanning, and bounds the cache via LRU. The index is a
 * reconstructable convenience — a failed flush is non-fatal.
 */
type CacheEntry = {
  id: string;
  revision: number;
  aspectWidth: number;
  aspectHeight: number;
  fileName: string;
  byteCount: number;
  lastAccessAt: number;
};

class CapsuleThumbnailDiskStore {
  private index: Map<string, CacheEntry> = new Map();
  private warmed = false;
  // Coalesces concurrent requests for the same key into one render.
  private inflight = new Map<string, Promise<DesktopCapsuleThumbnailResult>>();
  // Serialize index reads/writes so concurrent renders don't clobber it.
  private chain: Promise<void> = Promise.resolve();

  private directory(): string {
    return join(app.getPath("userData"), CACHE_SUBDIR);
  }

  private indexPath(): string {
    return join(this.directory(), "index.json");
  }

  private fileName(token: string): string {
    const hash = createHash("sha256").update(token).digest("hex").slice(0, 32);
    return `${hash}.png`;
  }

  private async warm(): Promise<void> {
    if (this.warmed) {
      return;
    }
    this.warmed = true;
    try {
      const raw = await fs.readFile(this.indexPath(), "utf8");
      const parsed = JSON.parse(raw) as Record<string, CacheEntry>;
      this.index = new Map(Object.entries(parsed));
    } catch {
      this.index = new Map();
    }
  }

  /** Run a mutation serialized after any in-flight index work. */
  private serialize<T>(work: () => Promise<T>): Promise<T> {
    const run = this.chain.then(work, work);
    // Keep the chain alive but swallow rejections so one failure doesn't wedge it.
    this.chain = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  private async flushIndex(): Promise<void> {
    try {
      await fs.mkdir(this.directory(), { recursive: true });
      const obj = Object.fromEntries(this.index.entries());
      const tmp = `${this.indexPath()}.${process.pid}.tmp`;
      await fs.writeFile(tmp, JSON.stringify(obj));
      await fs.rename(tmp, this.indexPath());
    } catch {
      // Index is reconstructable; a failed flush is non-fatal.
    }
  }

  /** Cached PNG bytes for a token, or null on a miss / vanished file (self-healing). */
  async read(token: string): Promise<Buffer | null> {
    return this.serialize(async () => {
      await this.warm();
      const entry = this.index.get(token);
      if (!entry) {
        return null;
      }
      const filePath = join(this.directory(), entry.fileName);
      try {
        const buffer = await fs.readFile(filePath);
        if (buffer.length === 0) {
          throw new Error("empty");
        }
        entry.lastAccessAt = Date.now();
        this.index.set(token, entry);
        return buffer;
      } catch {
        this.index.delete(token);
        await this.flushIndex();
        return null;
      }
    });
  }

  async write(
    token: string,
    id: string,
    revision: number,
    rendition: CapsuleThumbnailRendition,
    png: Buffer,
  ): Promise<void> {
    return this.serialize(async () => {
      await this.warm();
      const fileName = this.fileName(token);
      const filePath = join(this.directory(), fileName);
      try {
        await fs.mkdir(this.directory(), { recursive: true });
        const tmp = `${filePath}.${process.pid}.tmp`;
        await fs.writeFile(tmp, png);
        await fs.rename(tmp, filePath);
        this.index.set(token, {
          id: id.trim(),
          revision,
          aspectWidth: Math.max(1, Math.trunc(rendition.aspectWidth)),
          aspectHeight: Math.max(1, Math.trunc(rendition.aspectHeight)),
          fileName,
          byteCount: png.length,
          lastAccessAt: Date.now(),
        });
        await this.pruneToLimits();
        await this.flushIndex();
      } catch {
        // Best-effort cache: a failed write just means the next sighting re-renders.
      }
    });
  }

  /** Evict every cached rendition/revision of one capsule (a `/serve` 404). */
  async evictCapsule(capsuleId: string): Promise<void> {
    const id = capsuleId.trim();
    if (!id) {
      return;
    }
    return this.serialize(async () => {
      await this.warm();
      let changed = false;
      for (const [token, entry] of [...this.index.entries()]) {
        if (entry.id === id) {
          await this.removeFile(entry.fileName);
          this.index.delete(token);
          changed = true;
        }
      }
      if (changed) {
        await this.flushIndex();
      }
    });
  }

  private async removeFile(fileName: string): Promise<void> {
    await fs
      .rm(join(this.directory(), fileName), { force: true })
      .catch(() => undefined);
  }

  /** LRU eviction to the byte/record cap (oldest last-access first). */
  private async pruneToLimits(): Promise<void> {
    let totalBytes = 0;
    for (const entry of this.index.values()) {
      totalBytes += entry.byteCount;
    }
    if (this.index.size <= MAX_CACHE_RECORDS && totalBytes <= MAX_CACHE_BYTES) {
      return;
    }
    const ordered = [...this.index.entries()].sort(
      (a, b) => a[1].lastAccessAt - b[1].lastAccessAt,
    );
    for (const [token, entry] of ordered) {
      if (
        this.index.size <= MAX_CACHE_RECORDS &&
        totalBytes <= MAX_CACHE_BYTES
      ) {
        break;
      }
      await this.removeFile(entry.fileName);
      this.index.delete(token);
      totalBytes -= entry.byteCount;
    }
  }

  /** Track an in-flight render so concurrent callers share one render+cache. */
  trackInflight(
    token: string,
    factory: () => Promise<DesktopCapsuleThumbnailResult>,
  ): Promise<DesktopCapsuleThumbnailResult> {
    const existing = this.inflight.get(token);
    if (existing) {
      return existing;
    }
    const task = factory();
    this.inflight.set(token, task);
    return task.finally(() => this.inflight.delete(token));
  }
}

const diskStore = new CapsuleThumbnailDiskStore();

/**
 * Resolve a capsule thumbnail by `(id, revision, rendition)`: disk cache →
 * render-once. A `/serve` 404 surfaces as `{ status: 'deleted' }` and evicts
 * every cached rendition/revision of the capsule; transient failures surface as
 * `{ status: 'error' }` and stay retryable (never mislabeled deleted).
 */
export async function renderCapsuleThumbnail(
  settings: DesktopSettings,
  capsuleId: string,
  revision: number,
  rendition: CapsuleThumbnailRendition,
): Promise<DesktopCapsuleThumbnailResult> {
  const id = capsuleId?.trim() || "";
  if (!id) {
    return { status: "error", message: "capsuleId is required" };
  }
  const token = capsuleThumbnailStorageToken(id, revision, rendition);

  return diskStore.trackInflight(token, async () => {
    const cached = await diskStore.read(token);
    if (cached) {
      return { status: "ok", dataUrl: pngBufferToDataUrl(cached) };
    }

    let htmlResult;
    try {
      htmlResult = await getCapsuleHtml(settings, id);
    } catch (error) {
      return {
        status: "error",
        message: error instanceof Error ? error.message : String(error),
      };
    }
    if (htmlResult.status === "deleted") {
      await diskStore.evictCapsule(id);
      return { status: "deleted" };
    }

    const png = await renderThumbnailPng(htmlResult.html, rendition);
    if (!png) {
      return { status: "error", message: "Failed to render Capsule thumbnail" };
    }
    await diskStore.write(token, id, revision, rendition, png);
    return { status: "ok", dataUrl: pngBufferToDataUrl(png) };
  });
}

/** Evict every cached rendition/revision of one capsule (explicit delete / 404). */
export async function evictCapsuleThumbnails(capsuleId: string): Promise<void> {
  await diskStore.evictCapsule(capsuleId);
}
