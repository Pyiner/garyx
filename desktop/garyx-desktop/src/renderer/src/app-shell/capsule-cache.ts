import { capsuleHtmlStore } from './capsule-html-store';
import { capsuleThumbnailStore } from './capsule-thumbnail-store';

/**
 * Cross-wire the two capsule caches so a `/serve` 404 discovered by either store
 * tombstones the other for the same capsule id.
 *
 * The focused preview reads the HTML store (a live iframe) and the gallery/chat
 * cards read the thumbnail store (a cached PNG); they cache the same capsule
 * independently. Without this bridge, a capsule deleted out from under one
 * surface (e.g. the focused preview force-refreshes to a 404) would clear that
 * surface while the other kept serving a stale document/image. The explicit
 * delete button already invalidates both stores directly; this closes the
 * remaining 404-discovered paths.
 *
 * Injected via each store's `setCrossInvalidate` rather than a direct import so
 * the stores stay decoupled (no import cycle). `invalidateCapsule` does not emit
 * the cross hook, so the two stores cannot ping-pong. Idempotent.
 */
export function wireCapsuleCacheInvalidation(): void {
  capsuleHtmlStore.setCrossInvalidate((id) => capsuleThumbnailStore.invalidateCapsule(id));
  capsuleThumbnailStore.setCrossInvalidate((id) => capsuleHtmlStore.invalidateCapsule(id));
}

// Wire on module load so importing this module anywhere activates the bridge.
wireCapsuleCacheInvalidation();
