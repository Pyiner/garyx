import test from 'node:test';
import assert from 'node:assert/strict';

import {
  __resetCapsuleThumbnailStoreForTest,
  __setCapsuleThumbnailFetcherForTest,
  capsuleThumbnailCacheKey,
  capsuleThumbnailStore,
  CHAT_CARD_RENDITION,
  GALLERY_RENDITION,
} from './capsule-thumbnail-store.ts';

// Controlled fetcher: each call parks a {id, revision, rendition, resolve,
// reject} so the test drives completion timing and asserts concurrency /
// staleness / rendition behavior deterministically.
function makeController() {
  const calls = [];
  const fetcher = (id, revision, rendition) =>
    new Promise((resolve, reject) => {
      calls.push({ id, revision, rendition, resolve, reject });
    });
  return { calls, fetcher };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function stateOf(id, revision, rendition) {
  return capsuleThumbnailStore.getState(
    capsuleThumbnailCacheKey(id, revision, rendition),
  );
}

test.beforeEach(() => {
  __resetCapsuleThumbnailStoreForTest();
});

test('caps concurrent renders at 4 and drains the queue as slots free', async () => {
  const { calls, fetcher } = makeController();
  __setCapsuleThumbnailFetcherForTest(fetcher);

  for (let i = 0; i < 6; i += 1) {
    capsuleThumbnailStore.request(`id${i}`, 1, GALLERY_RENDITION, {});
  }
  assert.equal(calls.length, 4);
  assert.equal(capsuleThumbnailStore.__activeCount(), 4);
  assert.equal(stateOf('id5', 1, GALLERY_RENDITION).status, 'loading');

  calls[0].resolve({ status: 'ok', dataUrl: 'data:image/png;base64,AAA' });
  await flush();
  assert.equal(calls.length, 5);
  assert.equal(stateOf('id0', 1, GALLERY_RENDITION).status, 'ready');
});

test('dedupes concurrent requests for the same key into one render', async () => {
  const { calls, fetcher } = makeController();
  __setCapsuleThumbnailFetcherForTest(fetcher);

  capsuleThumbnailStore.request('id', 2, GALLERY_RENDITION, {});
  capsuleThumbnailStore.request('id', 2, GALLERY_RENDITION, {});
  capsuleThumbnailStore.request('id', 2, GALLERY_RENDITION, {});
  assert.equal(calls.length, 1);
});

test('keys by rendition so 16:10 and 16:9 are distinct cached images', async () => {
  const { calls, fetcher } = makeController();
  __setCapsuleThumbnailFetcherForTest(fetcher);

  // Same id + revision, different rendition → two independent renders/entries.
  capsuleThumbnailStore.request('id', 3, GALLERY_RENDITION, {});
  capsuleThumbnailStore.request('id', 3, CHAT_CARD_RENDITION, {});
  assert.equal(calls.length, 2);
  assert.deepEqual(calls[0].rendition, GALLERY_RENDITION);
  assert.deepEqual(calls[1].rendition, CHAT_CARD_RENDITION);

  calls[0].resolve({ status: 'ok', dataUrl: 'data:image/png;base64,GALLERY' });
  calls[1].resolve({ status: 'ok', dataUrl: 'data:image/png;base64,CHAT' });
  await flush();

  assert.deepEqual(stateOf('id', 3, GALLERY_RENDITION), {
    status: 'ready',
    dataUrl: 'data:image/png;base64,GALLERY',
  });
  assert.deepEqual(stateOf('id', 3, CHAT_CARD_RENDITION), {
    status: 'ready',
    dataUrl: 'data:image/png;base64,CHAT',
  });
});

test('serves cached image without re-rendering, and force re-renders', async () => {
  const { calls, fetcher } = makeController();
  __setCapsuleThumbnailFetcherForTest(fetcher);

  capsuleThumbnailStore.request('id', 1, GALLERY_RENDITION, {});
  calls[0].resolve({ status: 'ok', dataUrl: 'data:image/png;base64,CACHED' });
  await flush();
  assert.deepEqual(stateOf('id', 1, GALLERY_RENDITION), {
    status: 'ready',
    dataUrl: 'data:image/png;base64,CACHED',
  });

  // Cache hit: no new render.
  capsuleThumbnailStore.request('id', 1, GALLERY_RENDITION, {});
  assert.equal(calls.length, 1);

  // Force: re-render the same key.
  capsuleThumbnailStore.request('id', 1, GALLERY_RENDITION, { force: true });
  assert.equal(calls.length, 2);
  calls[1].resolve({ status: 'ok', dataUrl: 'data:image/png;base64,FRESH' });
  await flush();
  assert.deepEqual(stateOf('id', 1, GALLERY_RENDITION), {
    status: 'ready',
    dataUrl: 'data:image/png;base64,FRESH',
  });
});

test('maps deleted results and keeps transient failures retryable', async () => {
  const { calls, fetcher } = makeController();
  __setCapsuleThumbnailFetcherForTest(fetcher);

  capsuleThumbnailStore.request('gone', 1, GALLERY_RENDITION, {});
  calls[0].resolve({ status: 'deleted' });
  await flush();
  assert.equal(stateOf('gone', 1, GALLERY_RENDITION).status, 'deleted');

  // Structured render error from the main process: retryable, not a tombstone.
  capsuleThumbnailStore.request('flaky', 1, GALLERY_RENDITION, {});
  calls[1].resolve({ status: 'error', message: 'render failed' });
  await flush();
  const structured = stateOf('flaky', 1, GALLERY_RENDITION);
  assert.equal(structured.status, 'error');
  assert.match(structured.message, /render failed/);

  // A rejected promise (offline/transport) is also retryable error, not deleted.
  capsuleThumbnailStore.request('offline', 1, GALLERY_RENDITION, {});
  calls[2].reject(new Error('network down'));
  await flush();
  const thrown = stateOf('offline', 1, GALLERY_RENDITION);
  assert.equal(thrown.status, 'error');
  assert.match(thrown.message, /network down/);
});

test('delete while inflight: late result discarded, stays deleted across renditions', async () => {
  const { calls, fetcher } = makeController();
  __setCapsuleThumbnailFetcherForTest(fetcher);

  capsuleThumbnailStore.request('victim', 3, GALLERY_RENDITION, {});
  capsuleThumbnailStore.request('victim', 3, CHAT_CARD_RENDITION, {});
  assert.equal(stateOf('victim', 3, GALLERY_RENDITION).status, 'loading');

  // Capsule deleted while renders are in flight: every rendition tombstones.
  capsuleThumbnailStore.invalidateCapsule('victim');
  assert.equal(stateOf('victim', 3, GALLERY_RENDITION).status, 'deleted');
  assert.equal(stateOf('victim', 3, CHAT_CARD_RENDITION).status, 'deleted');

  // The in-flight renders resolve late: generation guard drops them so we never
  // write an image back over the tombstone.
  calls[0].resolve({ status: 'ok', dataUrl: 'data:image/png;base64,STALE' });
  calls[1].resolve({ status: 'ok', dataUrl: 'data:image/png;base64,STALE2' });
  await flush();
  assert.equal(stateOf('victim', 3, GALLERY_RENDITION).status, 'deleted');
  assert.equal(stateOf('victim', 3, CHAT_CARD_RENDITION).status, 'deleted');
});

test('stale completion still frees its slot so the queue keeps draining', async () => {
  const { calls, fetcher } = makeController();
  __setCapsuleThumbnailFetcherForTest(fetcher);

  for (const id of ['a', 'b', 'c', 'd']) {
    capsuleThumbnailStore.request(id, 1, GALLERY_RENDITION, {});
  }
  capsuleThumbnailStore.request('e', 1, GALLERY_RENDITION, {}); // queued behind 4
  assert.equal(calls.length, 4);

  capsuleThumbnailStore.invalidateCapsule('a');
  calls[0].resolve({ status: 'ok', dataUrl: 'data:image/png;base64,DISCARDED' });
  await flush();

  assert.equal(calls.length, 5);
  assert.equal(calls[4].id, 'e');
  assert.equal(stateOf('a', 1, GALLERY_RENDITION).status, 'deleted');
});
