import test from 'node:test';
import assert from 'node:assert/strict';

import {
  __resetCapsuleHtmlStoreForTest,
  __setCapsuleHtmlFetcherForTest,
  capsuleHtmlCacheKey,
  capsuleHtmlStore,
} from './capsule-html-store.ts';
import {
  __resetCapsuleThumbnailStoreForTest,
  __setCapsuleThumbnailFetcherForTest,
  capsuleThumbnailCacheKey,
  capsuleThumbnailStore,
  CHAT_CARD_RENDITION,
  GALLERY_RENDITION,
} from './capsule-thumbnail-store.ts';

// Mirrors `capsule-cache.ts`'s `wireCapsuleCacheInvalidation` (replicated here
// rather than imported: that module value-imports the stores extensionlessly,
// which the bundler resolves but `node --test` cannot. The behavior under test
// is the stores' cross-invalidation hook, which this wiring exercises directly).
function wireCapsuleCacheInvalidation() {
  capsuleHtmlStore.setCrossInvalidate((id) => capsuleThumbnailStore.invalidateCapsule(id));
  capsuleThumbnailStore.setCrossInvalidate((id) => capsuleHtmlStore.invalidateCapsule(id));
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function htmlState(id, revision) {
  return capsuleHtmlStore.getState(capsuleHtmlCacheKey(id, revision));
}
function thumbState(id, revision, rendition) {
  return capsuleThumbnailStore.getState(
    capsuleThumbnailCacheKey(id, revision, rendition),
  );
}

test.beforeEach(() => {
  __resetCapsuleHtmlStoreForTest();
  __resetCapsuleThumbnailStoreForTest();
  // __reset clears the injected cross-invalidator, so re-wire after each reset.
  wireCapsuleCacheInvalidation();
});

// The focused-preview HTML `/serve` 404 must drop the gallery/chat thumbnails
// for the same id — the desktop counterpart of the iOS centralized 404 evict.
test('a /serve 404 in the HTML store tombstones every thumbnail rendition for that id', async () => {
  const thumbCalls = [];
  __setCapsuleThumbnailFetcherForTest(
    () => new Promise((resolve) => thumbCalls.push({ resolve })),
  );
  capsuleThumbnailStore.request('victim', 3, GALLERY_RENDITION, {});
  capsuleThumbnailStore.request('victim', 3, CHAT_CARD_RENDITION, {});
  thumbCalls[0].resolve({ status: 'ok', dataUrl: 'data:image/png;base64,GAL' });
  thumbCalls[1].resolve({ status: 'ok', dataUrl: 'data:image/png;base64,CHAT' });
  await flush();
  assert.equal(thumbState('victim', 3, GALLERY_RENDITION).status, 'ready');
  assert.equal(thumbState('victim', 3, CHAT_CARD_RENDITION).status, 'ready');

  // Focused preview re-validates HTML and discovers the capsule is gone.
  const htmlCalls = [];
  __setCapsuleHtmlFetcherForTest(
    () => new Promise((resolve) => htmlCalls.push({ resolve })),
  );
  capsuleHtmlStore.request('victim', 3, {});
  htmlCalls[0].resolve({ status: 'deleted' });
  await flush();

  assert.equal(htmlState('victim', 3).status, 'deleted');
  // Cross-invalidated: both thumbnail renditions flip to deleted, no stale PNG.
  assert.equal(thumbState('victim', 3, GALLERY_RENDITION).status, 'deleted');
  assert.equal(thumbState('victim', 3, CHAT_CARD_RENDITION).status, 'deleted');
});

// The reverse: a 404 discovered while rendering a thumbnail must drop the
// focused preview's cached HTML so a re-opened preview is not stale either.
test('a /serve 404 while rendering a thumbnail tombstones the HTML preview for that id', async () => {
  const htmlCalls = [];
  __setCapsuleHtmlFetcherForTest(
    () => new Promise((resolve) => htmlCalls.push({ resolve })),
  );
  capsuleHtmlStore.request('victim', 3, {});
  htmlCalls[0].resolve({ status: 'ok', html: '<p>stale</p>' });
  await flush();
  assert.equal(htmlState('victim', 3).status, 'ready');

  const thumbCalls = [];
  __setCapsuleThumbnailFetcherForTest(
    () => new Promise((resolve) => thumbCalls.push({ resolve })),
  );
  capsuleThumbnailStore.request('victim', 3, GALLERY_RENDITION, {});
  thumbCalls[0].resolve({ status: 'deleted' });
  await flush();

  assert.equal(thumbState('victim', 3, GALLERY_RENDITION).status, 'deleted');
  // Cross-invalidated: the HTML preview is tombstoned, not left serving stale HTML.
  assert.equal(htmlState('victim', 3).status, 'deleted');
});
