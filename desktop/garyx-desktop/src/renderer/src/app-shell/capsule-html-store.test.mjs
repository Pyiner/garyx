import test from 'node:test';
import assert from 'node:assert/strict';

import {
  __resetCapsuleHtmlStoreForTest,
  __setCapsuleHtmlFetcherForTest,
  capsuleHtmlCacheKey,
  capsuleHtmlStore,
} from './capsule-html-store.ts';

// Controlled fetcher: each call parks a {id, resolve, reject} so the test drives
// completion timing and asserts concurrency / staleness behavior deterministically.
function makeController() {
  const calls = [];
  const fetcher = (id) =>
    new Promise((resolve, reject) => {
      calls.push({ id, resolve, reject });
    });
  return { calls, fetcher };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function stateOf(id, revision) {
  return capsuleHtmlStore.getState(capsuleHtmlCacheKey(id, revision));
}

test.beforeEach(() => {
  __resetCapsuleHtmlStoreForTest();
});

test('caps concurrent fetches at 4 and drains the queue as slots free', async () => {
  const { calls, fetcher } = makeController();
  __setCapsuleHtmlFetcherForTest(fetcher);

  for (let i = 0; i < 6; i += 1) {
    capsuleHtmlStore.request(`id${i}`, 1, {});
  }
  // Only 4 in flight; the other 2 wait in the queue.
  assert.equal(calls.length, 4);
  assert.equal(capsuleHtmlStore.__activeCount(), 4);
  assert.equal(stateOf('id5', 1).status, 'loading');

  calls[0].resolve({ status: 'ok', html: 'a' });
  await flush();
  // A freed slot drains the next queued job.
  assert.equal(calls.length, 5);
  assert.equal(stateOf('id0', 1).status, 'ready');
});

test('dedupes concurrent requests for the same key into one fetch', async () => {
  const { calls, fetcher } = makeController();
  __setCapsuleHtmlFetcherForTest(fetcher);

  capsuleHtmlStore.request('id', 2, {});
  capsuleHtmlStore.request('id', 2, {});
  capsuleHtmlStore.request('id', 2, {});
  assert.equal(calls.length, 1);
});

test('serves cached HTML without refetching, and force refetches', async () => {
  const { calls, fetcher } = makeController();
  __setCapsuleHtmlFetcherForTest(fetcher);

  capsuleHtmlStore.request('id', 1, {});
  calls[0].resolve({ status: 'ok', html: 'cached' });
  await flush();
  assert.deepEqual(stateOf('id', 1), { status: 'ready', html: 'cached' });

  // Cache hit: no new fetch.
  capsuleHtmlStore.request('id', 1, {});
  assert.equal(calls.length, 1);

  // Force: re-fetch the same key.
  capsuleHtmlStore.request('id', 1, { force: true });
  assert.equal(calls.length, 2);
  calls[1].resolve({ status: 'ok', html: 'fresh' });
  await flush();
  assert.deepEqual(stateOf('id', 1), { status: 'ready', html: 'fresh' });
});

test('maps deleted results and keeps transient failures retryable', async () => {
  const { calls, fetcher } = makeController();
  __setCapsuleHtmlFetcherForTest(fetcher);

  capsuleHtmlStore.request('gone', 1, {});
  calls[0].resolve({ status: 'deleted' });
  await flush();
  assert.equal(stateOf('gone', 1).status, 'deleted');

  capsuleHtmlStore.request('flaky', 1, {});
  calls[1].reject(new Error('network down'));
  await flush();
  const state = stateOf('flaky', 1);
  assert.equal(state.status, 'error');
  assert.match(state.message, /network down/);
});

test('delete while inflight: late result is discarded and stays deleted', async () => {
  const { calls, fetcher } = makeController();
  __setCapsuleHtmlFetcherForTest(fetcher);

  capsuleHtmlStore.request('victim', 3, {});
  assert.equal(stateOf('victim', 3).status, 'loading');

  // Capsule deleted while the fetch is still in flight.
  capsuleHtmlStore.invalidateCapsule('victim');
  assert.equal(stateOf('victim', 3).status, 'deleted');

  // The in-flight fetch resolves late with the old HTML: generation guard drops
  // it so we never write deleted HTML back.
  calls[0].resolve({ status: 'ok', html: 'stale-html' });
  await flush();
  assert.equal(stateOf('victim', 3).status, 'deleted');
});

test('stale completion still frees its slot so the queue keeps draining', async () => {
  const { calls, fetcher } = makeController();
  __setCapsuleHtmlFetcherForTest(fetcher);

  for (const id of ['a', 'b', 'c', 'd']) {
    capsuleHtmlStore.request(id, 1, {});
  }
  capsuleHtmlStore.request('e', 1, {}); // queued behind the 4 in-flight
  assert.equal(calls.length, 4);

  capsuleHtmlStore.invalidateCapsule('a');
  // Stale resolution of 'a' must release the slot and let 'e' start.
  calls[0].resolve({ status: 'ok', html: 'discarded' });
  await flush();

  assert.equal(calls.length, 5);
  assert.equal(calls[4].id, 'e');
  assert.equal(stateOf('a', 1).status, 'deleted');
});

test('evicts old ready HTML after browsing a bounded number of capsules', async () => {
  __setCapsuleHtmlFetcherForTest(async (id) => ({
    status: 'ok',
    html: `<main>${id}</main>`,
  }));

  for (let index = 0; index < 257; index += 1) {
    capsuleHtmlStore.request(`history-${index}`, 1, {});
  }
  await flush();

  assert.equal(stateOf('history-0', 1).status, 'idle');
  assert.equal(stateOf('history-256', 1).status, 'ready');
});
