/**
 * Pure (no-Electron) helpers for rendering a Capsule into a thumbnail: the
 * device-width layout, the horizontal content-fill transform, the cache storage
 * token, and the render-schema version. Kept free of `electron` imports so the
 * logic is covered by headless `node --test` and mirrors the iOS
 * `GaryxMobileCore` layer (`GaryxCapsuleThumbnailRendering` + `…Fill`).
 *
 * #TASK-1458: the thumbnail must render like a phone full-screen view — at the
 * device logical width, with content filling the frame — never letterboxed
 * behind a painted backing. The old wide (1024pt) render viewport let an author
 * `max-width` container sit centered with white side gutters.
 */

export interface CapsuleThumbnailRendition {
  aspectWidth: number;
  aspectHeight: number;
}

/**
 * Version of the thumbnail *render output*. Bump whenever the renderer changes
 * how a capsule becomes a PNG (render viewport width, fill transform, crop,
 * backing) so every previously cached thumbnail — whose storage token embeds
 * this version — misses and re-renders instead of serving a stale (e.g. old
 * white-edged) image. Keep in sync with the iOS
 * `GaryxCapsuleThumbnailRenderSchema.version`.
 *
 * - `1`: original wide (1024pt) render viewport with an opaque dark backing.
 * - `2`: device-width (390pt) render + horizontal content-fill, no backing.
 */
export const CAPSULE_THUMBNAIL_SCHEMA_VERSION = 2;

/**
 * Standard device logical width (CSS px) the capsule is laid out into, so it
 * fills like a phone full-screen render rather than a desktop-wide one. Matches
 * `GaryxCapsuleThumbnailSnapshotPlan.deviceLayoutWidth` on iOS.
 */
export const CAPSULE_THUMBNAIL_DEVICE_WIDTH = 390;

export function renditionToken(rendition: CapsuleThumbnailRendition): string {
  const w = Math.max(1, Math.trunc(rendition.aspectWidth));
  const h = Math.max(1, Math.trunc(rendition.aspectHeight));
  return `${w}x${h}`;
}

/** Stable, filesystem-key token, e.g. `<id>.r3.16x10.s2`. */
export function capsuleThumbnailStorageToken(
  id: string,
  revision: number,
  rendition: CapsuleThumbnailRendition,
  schemaVersion: number = CAPSULE_THUMBNAIL_SCHEMA_VERSION,
): string {
  return `${id.trim()}.r${revision}.${renditionToken(rendition)}.s${schemaVersion}`;
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

/**
 * The horizontal transform that makes measured content fill the frame
 * flush-left, or `null` when it already fills (no left gutter, within 1px).
 * Mirrors the iOS `GaryxCapsuleThumbnailFill.fillTransform`; the in-page JS in
 * `capsuleThumbnailFillScript` applies the same arithmetic.
 */
export function fillTransform(
  contentLeft: number,
  contentWidth: number,
  viewportWidth: number,
): { scale: number; translateX: number } | null {
  if (!(viewportWidth > 0) || !(contentWidth > 0)) {
    return null;
  }
  const left = Math.max(0, contentLeft);
  const scale = viewportWidth / contentWidth;
  if (scale <= 1.005 && left <= 1) {
    return null;
  }
  return { scale, translateX: -left * scale };
}

/**
 * JS injected (after layout settles) that measures the visible content's
 * horizontal extent and applies `fillTransform` to `document.documentElement`,
 * so content fills the width with no side gutters and no injected backing.
 * Self-contained, never throws, a no-op when content already fills. The
 * arithmetic mirrors `fillTransform` (cross-engine verified — Electron is
 * Chromium and matches WKWebView CSS).
 */
export const capsuleThumbnailFillScript = `(function () {
  try {
    if (!document.body) { return; }
    var vw = window.innerWidth;
    if (!(vw > 0)) { return; }
    var minL = Infinity, maxR = -Infinity;
    var els = document.body.querySelectorAll('*');
    for (var i = 0; i < els.length; i++) {
      var el = els[i];
      var r = el.getBoundingClientRect();
      if (r.width <= 0 || r.height <= 0) { continue; }
      var cs = getComputedStyle(el);
      if (cs.visibility === 'hidden' || cs.display === 'none') { continue; }
      if (r.right <= 0 || r.left >= vw) { continue; }
      var l = Math.max(0, r.left);
      var rr = Math.min(vw, r.right);
      if (l < minL) { minL = l; }
      if (rr > maxR) { maxR = rr; }
    }
    if (!isFinite(minL) || !isFinite(maxR)) { return; }
    var left = Math.max(0, minL);
    var width = Math.max(1, maxR - left);
    var scale = vw / width;
    if (scale <= 1.005 && left <= 1) { return; }
    var root = document.documentElement;
    root.style.transformOrigin = 'top left';
    root.style.transform = 'translateX(' + (-left * scale) + 'px) scale(' + scale + ')';
  } catch (e) { /* best-effort fill */ }
})();`;

/**
 * Stale-schema purge decision (cache invalidation after a schema bump): given
 * each on-disk entry's stored token and the token the *current* schema would
 * produce for it, returns which tokens to keep vs evict. A render from a
 * previous schema (or a legacy token with no schema suffix) no longer matches
 * and is evicted. Mirrors iOS `GaryxCapsuleThumbnailCachePruner.evictingStaleSchema`.
 */
export function evictingStaleSchemaTokens(
  entries: Array<{ token: string; currentToken: string }>,
): { keep: string[]; evict: string[] } {
  const keep: string[] = [];
  const evict: string[] = [];
  for (const entry of entries) {
    if (entry.token === entry.currentToken) {
      keep.push(entry.token);
    } else {
      evict.push(entry.token);
    }
  }
  return { keep, evict };
}
